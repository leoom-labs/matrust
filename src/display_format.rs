//! Command-window display: variable echo (`x = ...`) and `disp`.
//!
//! The reference fixtures in `tests/fixtures/display/` cover 96 initial cases,
//! 207 first-probe cases and a second probe batch. `tests/display_format.rs`
//! replays them; behavior beyond the covered cases remains a hypothesis.
//!
//! Every function returns an *entry* without its last newline, because the
//! console adds one newline after each entry.
use crate::{
    Array, CellArray, CharacterRow, Complex64, ComplexMatrix, LogicalMatrix, Matrix, NumericClass,
    Result, TypedArray,
};
use std::fmt::Write;

/// Console width in characters when the host does not report one: the reference's
/// `CommandWindowSize` was 107 (P154; `1:20` splits after 17 six-wide columns).
pub const DEFAULT_WIDTH: usize = 107;
/// Longest text shown for one character row inside a cell or struct summary.
pub(crate) const MAX_TEXT_CHARS: usize = 1024;

/// How `format` writes numeric values: fixed point with a common scale
/// factor (`short`, `long`), `%g` (`shortG`, `longG`), `e` form (`shortE`,
/// `longE`) or engineering form (`shortEng`, `longEng`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Notation {
    #[default]
    Fixed,
    /// Fixed point or `e` form, whichever is more compact, with 5 (short) or
    /// 15 (long; 7 for single) significant digits and no trailing zeros.
    General,
    /// `e` form with 4 (short) or 15 (long; 7 for single) decimals.
    Scientific,
    /// Mantissa in `1 .. 1000` and an exponent that is a multiple of 3,
    /// written `12.3450e+003` (sign and three exponent digits). Decimals match
    /// `shortE`/`longE`.
    Engineering,
}

/// Session display settings: `format short|long` with an optional `g`/`e`,
/// `format compact|loose` and the console width in characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayFormat {
    pub long: bool,
    pub notation: Notation,
    pub compact: bool,
    pub width: usize,
    /// Maximum bytes in one displayed value, supplied by the host limits.
    pub max_bytes: usize,
    /// The host reported its console width, so help prose wraps to it.
    pub wrap_help: bool,
}

impl Default for DisplayFormat {
    fn default() -> Self {
        Self {
            long: false,
            notation: Notation::Fixed,
            compact: false,
            width: DEFAULT_WIDTH,
            max_bytes: 2 * 1024 * 1024,
            wrap_help: false,
        }
    }
}

/// Numeric layout constants of one `format` precision.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Style {
    decimals: usize,
    /// Most integer digits a fixed-point value may have (`999.9999`, `99.9...`).
    fixed_digits: i32,
    fixed_width: usize,
    exp_width: usize,
    /// Significant digits a lone value is rounded to before its integer digits
    /// are counted (long: 15, so `99.99999999999999` is `9.999...e+01`, QL53);
    /// `None` counts them after rounding to `decimals`.
    significant: Option<i32>,
    /// Real part of a complex value in `e` form (long: `      1.005...e+02`, QL48).
    complex_exp_re_width: usize,
    /// Fields of a fixed-point complex element `re +im i`: the real part
    /// (`   1.0000`) and the imaginary part after its sign (` 2.0000`, `20.0000`).
    complex_re_width: usize,
    complex_im_width: usize,
    notation: Notation,
    /// Significant digits of `Notation::General` (`%.5g`, `%.15g`, single `%.7g`).
    general_digits: usize,
}

/// Most integer digits of a fixed-point complex part (`10.0000 +20.0000i`);
/// a lone complex value with more prints in `e` form, a matrix takes a factor.
const COMPLEX_FIXED_DIGITS: i32 = 2;

const SHORT: Style = Style {
    decimals: 4,
    fixed_digits: 3,
    fixed_width: 10,
    exp_width: 13,
    significant: None,
    complex_exp_re_width: 13,
    complex_re_width: 9,
    complex_im_width: 7,
    notation: Notation::Fixed,
    general_digits: 5,
};
/// `format long` for single: 7 decimals (`3.1415927`).
const SINGLE_LONG: Style = Style {
    decimals: 7,
    fixed_digits: 2,
    fixed_width: 12,
    exp_width: 16,
    significant: None,
    complex_exp_re_width: 16,
    complex_re_width: 11,
    complex_im_width: 10,
    notation: Notation::Fixed,
    general_digits: 7,
};
const LONG: Style = Style {
    decimals: 15,
    fixed_digits: 2,
    fixed_width: 20,
    exp_width: 26,
    significant: Some(15),
    complex_exp_re_width: 27,
    complex_re_width: 19,
    complex_im_width: 18,
    notation: Notation::Fixed,
    general_digits: 15,
};

impl DisplayFormat {
    pub(crate) fn style(&self) -> Style {
        let base = if self.long { LONG } else { SHORT };
        Style {
            notation: self.notation,
            ..base
        }
    }

    /// Style of `single` values: 7 decimals or digits under `long`; a lone
    /// short value in `e` form is two characters narrower (`  3.4028e+38`).
    pub(crate) fn single_style(&self) -> Style {
        let base = if self.long {
            SINGLE_LONG
        } else {
            Style {
                exp_width: 12,
                ..SHORT
            }
        };
        Style {
            notation: self.notation,
            ..base
        }
    }

    /// Apply one `format` option; `None` resets to `short` and `loose`.
    pub fn apply_option(&mut self, option: Option<&str>) -> Result<()> {
        self.apply_options(option.as_slice())
    }

    /// Apply `format`'s option words, case-insensitive. None (or `default`)
    /// resets to `short` and `loose`; `short`/`long` with an optional `g`, `e`
    /// or `eng` set the numeric style in any spelling (`longG`, `long g`,
    /// `shortEng`, `format('short','eng')`, `format("long G")`); `compact`/
    /// `loose` set the line spacing alone. A rejected word leaves `self` unchanged.
    pub fn apply_options(&mut self, options: &[&str]) -> Result<()> {
        let mut words = Vec::with_capacity(2);
        for option in options {
            let parts: Vec<String> = option
                .split_whitespace()
                .map(str::to_ascii_lowercase)
                .collect();
            if parts.is_empty() {
                return Err(format_option_error(options));
            }
            words.extend(parts);
        }
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        let style = match words.as_slice() {
            [] | ["default"] => {
                *self = DisplayFormat {
                    long: false,
                    notation: Notation::Fixed,
                    compact: false,
                    ..*self
                };
                return Ok(());
            }
            ["compact"] => {
                self.compact = true;
                return Ok(());
            }
            ["loose"] => {
                self.compact = false;
                return Ok(());
            }
            [one] => one
                .strip_prefix("short")
                .map(|rest| (false, rest))
                .or_else(|| one.strip_prefix("long").map(|rest| (true, rest))),
            [size @ ("short" | "long"), kind @ ("g" | "e" | "eng")] => {
                Some((*size == "long", *kind))
            }
            _ => None,
        };
        let notation = match style {
            Some((_, "")) => Notation::Fixed,
            Some((_, "g")) => Notation::General,
            Some((_, "e")) => Notation::Scientific,
            Some((_, "eng")) => Notation::Engineering,
            _ => return Err(format_option_error(options)),
        };
        self.long = style.is_some_and(|(long, _)| long);
        self.notation = notation;
        Ok(())
    }
}

fn format_option_error(options: &[&str]) -> String {
    format!(
        "format: unsupported option '{}'; use short, long, shortG, longG, shortE, longE, \
         shortEng, longEng (or two words: short or long, then g, e or eng), compact, loose \
         or default",
        options.join(" ")
    )
}

/// Decimal exponent of a finite non-zero value: `1234.5` is 3, `0.001` is -3.
fn exponent10(x: f64) -> i32 {
    let text = format!("{x:e}");
    text.rsplit('e')
        .next()
        .and_then(|e| e.parse().ok())
        .unwrap_or(0)
}

/// Integer digits as MATLAB counts them: `100.5` has 3, `0.5` has 0, `0.001` has -2.
fn digits(max_abs: f64) -> i32 {
    if max_abs == 0. {
        1
    } else {
        exponent10(max_abs) + 1
    }
}

/// C-style `%.<precision>e` (at least two exponent digits).
pub(crate) fn exponent(v: f64, precision: usize) -> String {
    let text = format!("{v:.precision$e}");
    let (mantissa, exp) = text.split_once('e').unwrap_or((&text, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    format!(
        "{mantissa}e{}{:02}",
        if exp < 0 { '-' } else { '+' },
        exp.abs()
    )
}

/// Decimal digits of a finite `|v|` rounded to 17 significant figures, and the
/// power of ten of the first one. Reference formatting rounds through
/// these 17 digits, so `1.70085`, stored as 1.70084999999999997..., prints as
/// `1.7009` (`(1:25)*1000.5` with its `1.0e+04 *` factor).
fn digits17(v: f64) -> (Vec<u8>, i32) {
    let text = format!("{:.16e}", v.abs());
    let (mantissa, exp) = text.split_once('e').unwrap_or((&text, "0"));
    (
        mantissa
            .bytes()
            .filter(u8::is_ascii_digit)
            .map(|b| b - b'0')
            .collect(),
        exp.parse().unwrap_or(0),
    )
}

/// The first `keep` digits, rounded half up on the next one; `true` when the
/// rounding carried into a new leading digit (`9.99996` to `10.0000`).
fn round_half_up(digits: &[u8], keep: i32) -> (Vec<u8>, bool) {
    let n = usize::try_from(keep).unwrap_or(0);
    let mut kept: Vec<u8> = (0..n)
        .map(|i| digits.get(i).copied().unwrap_or(0))
        .collect();
    if keep < 0 || digits.get(n).is_none_or(|&d| d < 5) {
        return (kept, false);
    }
    for d in kept.iter_mut().rev() {
        if *d < 9 {
            *d += 1;
            return (kept, false);
        }
        *d = 0;
    }
    kept.insert(0, 1);
    (kept, true)
}

/// `%.<decimals>f` of a finite value, rounded as the reference does (see `digits17`).
fn fixed(v: f64, decimals: usize) -> String {
    let (digits, exp) = digits17(v);
    let (mut kept, _) = round_half_up(&digits, exp + 1 + decimals as i32);
    if kept.len() <= decimals {
        kept.splice(0..0, std::iter::repeat_n(0, decimals + 1 - kept.len()));
    }
    let point = kept.len() - decimals;
    let mut text = String::with_capacity(kept.len() + 2);
    if v < 0. {
        text.push('-');
    }
    for (i, d) in kept.iter().enumerate() {
        if i == point {
            text.push('.');
        }
        text.push(char::from(b'0' + d));
    }
    text
}

/// `%.<decimals>e` of a finite value, rounded as the reference does (see `digits17`).
fn exp_form(v: f64, decimals: usize) -> String {
    let (digits, mut exp) = digits17(v);
    let (mut kept, carried) = round_half_up(&digits, decimals as i32 + 1);
    if carried {
        kept.pop();
        exp += 1;
    }
    let mut text = String::with_capacity(decimals + 8);
    if v < 0. {
        text.push('-');
    }
    for (i, d) in kept.iter().enumerate() {
        if i == 1 {
            text.push('.');
        }
        text.push(char::from(b'0' + d));
    }
    text.push_str(&format!(
        "e{}{:02}",
        if exp < 0 { '-' } else { '+' },
        exp.abs()
    ));
    text
}

/// Engineering exponent: the multiple of 3 at or below `exp` that leaves 1, 2
/// or 3 digits before the decimal point. Negative `exp` does not use Rust's
/// truncating remainder.
fn engineering_exponent(exp: i32) -> i32 {
    if exp < 0 {
        exp - 2 + ((-exp + 2) % 3)
    } else {
        exp - (exp % 3)
    }
}

/// `12.3450e+003`, `12.3457e-003`; zero is a bare `0`, as in the reference. The
/// exponent (sign and at least three digits) is a multiple of 3 and the
/// mantissa is in `1 .. 1000` after rounding. Digits come from `digits17`, the
/// same half-up path as `fixed` and `exp_form`. No power of ten is formed in
/// `f64`, so a subnormal stays finite and non-zero instead of becoming `Inf` or `0`.
fn engineering_form(v: f64, decimals: usize) -> String {
    if v == 0. {
        return "0".into();
    }
    let (digits, exp) = digits17(v);
    let mut eng = engineering_exponent(exp);
    let mut before = exp - eng + 1;
    let (mut kept, carried) = round_half_up(&digits, before + decimals as i32);
    if carried {
        before += 1;
        if before >= 4 {
            eng += 3;
            before = 1;
            kept.truncate(before as usize + decimals);
        }
    }
    let mut text = String::with_capacity(kept.len() + 16);
    if v < 0. {
        text.push('-');
    }
    let point = before as usize;
    for (i, d) in kept.iter().enumerate() {
        if i == point {
            text.push('.');
        }
        text.push(char::from(b'0' + d));
    }
    text.push('e');
    text.push(if eng < 0 { '-' } else { '+' });
    text.push_str(&format!("{:03}", eng.unsigned_abs()));
    text
}

/// `%.<digits>g` of a finite value, rounded as the reference does (see
/// `digits17`): fixed point while the rounded decimal exponent is in
/// `-4..digits`, else `e` form; trailing zeros and a bare point are dropped
/// (`3.1416`, `1000.5`, `1e-05`, `1.2346e+05`, `0.3`).
fn general_form(v: f64, digits: usize) -> String {
    if v == 0. {
        return "0".into();
    }
    let (all, exp) = digits17(v);
    let (_, carried) = round_half_up(&all, digits as i32);
    let exp = exp + i32::from(carried);
    if exp < -4 || exp >= digits as i32 {
        let text = exp_form(v, digits - 1);
        let (mantissa, power) = text.split_once('e').unwrap_or((&text, ""));
        format!("{}e{power}", without_trailing_zeros(mantissa))
    } else {
        let decimals = usize::try_from(digits as i32 - 1 - exp).unwrap_or(0);
        without_trailing_zeros(&fixed(v, decimals)).to_owned()
    }
}

fn without_trailing_zeros(text: &str) -> &str {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text
    }
}

/// A finite value in `shortG`/`longG`/`shortE`/`longE`/`shortEng`/`longEng`.
fn styled_text(v: f64, st: &Style) -> String {
    match st.notation {
        Notation::General => general_form(v, st.general_digits),
        Notation::Engineering => engineering_form(v, st.decimals),
        _ => exp_form(v, st.decimals),
    }
}

fn non_finite(v: f64) -> Option<&'static str> {
    if v.is_nan() {
        Some("NaN")
    } else if v.is_infinite() {
        Some(if v > 0. { "Inf" } else { "-Inf" })
    } else {
        None
    }
}

fn integer_text(v: f64) -> String {
    format!("{}", v as i64)
}

/// A value that prints without decimals: whole and below 1e9.
fn is_whole(v: f64) -> bool {
    v == v.trunc() && v.abs() < 1e9
}

/// How a real value is shown on its own (struct field, cell element, scalar echo).
#[derive(PartialEq)]
enum Single {
    Whole,
    Fixed,
    Exp,
    /// `shortG`/`longG`/`shortE`/`longE`: `styled_text` in the `e` form width.
    Styled,
}

/// Whether a lone magnitude prints in fixed point: above 0.001 (`0.001` itself is
/// `1.0000e-03`, `0.0020` is fixed) and at most `max_digits` integer digits once
/// rounded (`999.99999` is `1.0000e+03`, `9.99996` is `10.0000`).
fn fits_fixed(a: f64, max_digits: i32, st: &Style) -> bool {
    a > 1e-3 && rounded_digits(a, st) <= max_digits
}

/// Integer digits of a finite `a >= 0` once rounded for display.
fn rounded_digits(a: f64, st: &Style) -> i32 {
    let text = fixed(a, st.decimals);
    let integer = text.split('.').next().unwrap_or(&text).len() as i32;
    match st.significant {
        Some(keep) if a >= 1. => {
            let (digits, exp) = digits17(a);
            let (_, carried) = round_half_up(&digits, keep);
            integer.max(exp + 1 + i32::from(carried))
        }
        _ => integer,
    }
}

fn single_kind(v: f64, st: &Style) -> Single {
    if !v.is_finite() {
        Single::Whole
    } else if st.notation == Notation::Engineering {
        // Whole numbers stay in engineering form (`12345` is `12.3450e+003`).
        Single::Styled
    } else if is_whole(v) {
        Single::Whole
    } else if st.notation != Notation::Fixed {
        Single::Styled
    } else if fits_fixed(v.abs(), st.fixed_digits, st) {
        Single::Fixed
    } else {
        Single::Exp
    }
}

/// One real number as a scalar prints: `6`, `3.1416`, `1.2346e+04`, `NaN`.
pub(crate) fn scalar_text(v: f64, st: &Style) -> String {
    if let Some(text) = non_finite(v) {
        return text.into();
    }
    match single_kind(v, st) {
        Single::Whole => integer_text(v),
        Single::Fixed => fixed(v, st.decimals),
        Single::Exp => exp_form(v, st.decimals),
        Single::Styled => styled_text(v, st),
    }
}

/// One complex number as a scalar prints: `3.0000 + 4.0000i`.
pub(crate) fn complex_scalar_text(z: Complex64, st: &Style) -> String {
    let grid = complex_grid(1, 1, &[z], st);
    grid.cells[0].trim_start().to_owned()
}

/// Cells of equal width, column-major, plus the optional common scale factor.
struct Grid {
    rows: usize,
    cols: usize,
    width: usize,
    cells: Vec<String>,
    scale: Option<i32>,
}

fn pad_left(text: &str, width: usize) -> String {
    format!("{text:>width$}")
}

fn output_too_large(fmt: &DisplayFormat) -> String {
    if fmt.max_bytes == 2 * 1024 * 1024 {
        "Execution output exceeds 2000 entries or 2 MiB; use semicolons to suppress output".into()
    } else {
        format!(
            "Displayed value exceeds {} MiB; use semicolons to suppress output",
            fmt.max_bytes / (1024 * 1024)
        )
    }
}

fn checked(text: String, fmt: &DisplayFormat) -> Result<String> {
    check_size(text.len(), 1, fmt)?;
    Ok(text)
}

fn check_size(count: usize, width: usize, fmt: &DisplayFormat) -> Result<()> {
    if count.saturating_mul(width) > fmt.max_bytes {
        Err(output_too_large(fmt))
    } else {
        Ok(())
    }
}

/// Largest finite magnitude and whether every finite value is whole.
fn survey(data: impl Iterator<Item = f64>) -> (f64, bool) {
    let mut max_abs = 0f64;
    let mut whole = true;
    for v in data.filter(|v| v.is_finite()) {
        max_abs = max_abs.max(v.abs());
        whole &= v == v.trunc();
    }
    (max_abs, whole)
}

/// Common factor of a matrix: `1.0e+03 *` puts the largest magnitude in 1..10
/// when, rounded, it has more than `max_digits` integer digits (`[999.99999 1]`
/// is `1.0e+03 *`, Q43). Up to 0.001 the factor is `10^ceil(log10(max))`:
/// `[0.0001 0.0002]` is `1.0e-03 *` / `0.1000 0.2000` (P39), `[0.001 0.0005]`
/// is `1.0e-03 *` / `1.0000 0.5000` (Q41), `[eps 2*eps]` is `1.0e-15 *` (P42).
fn scale_of(max_abs: f64, count: usize, max_digits: i32, st: &Style) -> Option<i32> {
    if count < 2 || max_abs == 0. {
        None
    } else if max_abs <= 1e-3 {
        let (digits, exp) = digits17(max_abs);
        let power_of_ten = digits.first() == Some(&1) && digits[1..].iter().all(|&d| d == 0);
        Some(if power_of_ten { exp } else { exp + 1 })
    } else {
        let d = rounded_digits(max_abs, st);
        (d > max_digits).then_some(d - 1)
    }
}

fn scale_value(v: f64, scale: Option<i32>) -> f64 {
    scale.map_or(v, |e| v / 10f64.powi(e))
}

/// A matrix element in fixed point: exact zero is a bare `0`.
fn fixed_text(v: f64, st: &Style) -> String {
    if let Some(text) = non_finite(v) {
        text.into()
    } else if v == 0. {
        "0".into()
    } else {
        fixed(v, st.decimals)
    }
}

fn real_grid(
    rows: usize,
    cols: usize,
    data: &[f64],
    st: &Style,
    fmt: &DisplayFormat,
) -> Result<Grid> {
    if st.notation == Notation::Engineering {
        // Sign, three digits, a point, the decimals, `e` and `-324`.
        let bound = 8 + st.decimals + 8;
        check_size(data.len(), bound, fmt)?;
        let texts: Vec<String> = data
            .iter()
            .map(|&v| match non_finite(v) {
                Some(text) => text.to_owned(),
                None => engineering_form(v, st.decimals),
            })
            .collect();
        let width = texts
            .iter()
            .map(|text| text.chars().count())
            .max()
            .unwrap_or(0)
            .saturating_add(2);
        let cells = texts.iter().map(|text| pad_left(text, width)).collect();
        return Ok(Grid {
            rows,
            cols,
            width,
            cells,
            scale: None,
        });
    }
    let (max_abs, whole) = survey(data.iter().copied());
    let d = digits(max_abs);
    if data.len() == 1 {
        let v = data[0];
        let (text, width) = match single_kind(v, st) {
            Single::Whole => (scalar_text(v, st), if d <= 3 { 6 } else { 12 }),
            Single::Fixed => (scalar_text(v, st), st.fixed_width),
            Single::Exp | Single::Styled => (scalar_text(v, st), st.exp_width),
        };
        let width = if non_finite(v).is_some() { 6 } else { width };
        return Ok(Grid {
            rows,
            cols,
            width,
            cells: vec![pad_left(&text, width)],
            scale: None,
        });
    }
    if whole && d <= 9 {
        let width = if d <= 3 { 6 } else { 12 };
        check_size(data.len(), width, fmt)?;
        let cells = data
            .iter()
            .map(|&v| {
                pad_left(
                    &non_finite(v).map_or_else(|| integer_text(v), str::to_owned),
                    width,
                )
            })
            .collect();
        return Ok(Grid {
            rows,
            cols,
            width,
            cells,
            scale: None,
        });
    }
    if st.notation != Notation::Fixed {
        // Integer-valued matrices stay integers (above); otherwise every
        // element in `%g` or `e` form, never with a common scale factor.
        check_size(data.len(), st.exp_width, fmt)?;
        let cells = data
            .iter()
            .map(|&v| {
                let text = match non_finite(v) {
                    Some(text) => text.into(),
                    None if v == 0. => "0".into(),
                    None => styled_text(v, st),
                };
                pad_left(&text, st.exp_width)
            })
            .collect();
        return Ok(Grid {
            rows,
            cols,
            width: st.exp_width,
            cells,
            scale: None,
        });
    }
    let scale = scale_of(max_abs, data.len(), st.fixed_digits, st);
    check_size(data.len(), st.fixed_width, fmt)?;
    let cells = data
        .iter()
        .map(|&v| pad_left(&fixed_text(scale_value(v, scale), st), st.fixed_width))
        .collect();
    Ok(Grid {
        rows,
        cols,
        width: st.fixed_width,
        cells,
        scale,
    })
}

/// One part of a complex element: never a bare `0` (`0.0000 + 1.0000i`)
/// except in `%g` form, where zero is `0`.
fn complex_part(v: f64, scale: Option<i32>, exp: bool, st: &Style) -> String {
    if let Some(text) = non_finite(v) {
        return text.into();
    }
    // `-0` prints as `0.0000`, as in a real matrix.
    let v = if v == 0. { 0. } else { v };
    if st.notation == Notation::Engineering {
        return engineering_form(v, st.decimals);
    }
    if st.notation == Notation::General {
        general_form(v, st.general_digits)
    } else if exp {
        exp_form(v, st.decimals)
    } else {
        fixed(scale_value(v, scale), st.decimals)
    }
}

/// Complex elements `re +im i` in fixed point; a lone value (scalar echo, cell
/// and struct summaries) that fixed point cannot show prints both parts in `e`
/// form (`1.0050e+02 + 2.0000e+00i`), a matrix takes a common factor instead.
/// `shortE`/`longE` print every element in `e` form and `shortG`/`longG` in
/// `%g` form, both in the `e` form widths and without a factor.
fn complex_grid(rows: usize, cols: usize, data: &[Complex64], st: &Style) -> Grid {
    if st.notation == Notation::Engineering {
        let parts: Vec<(String, bool, String)> = data
            .iter()
            .map(|z| {
                (
                    complex_part(z.re, None, true, st),
                    z.im < 0.,
                    complex_part(z.im.abs(), None, true, st),
                )
            })
            .collect();
        let re_width = parts
            .iter()
            .map(|part| part.0.chars().count())
            .max()
            .unwrap_or(0);
        // One pad keeps the sign off the digits (`+ 4.0000e+000i`), as `e` form does.
        let im_width = parts
            .iter()
            .map(|part| part.2.chars().count())
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let cells = parts
            .into_iter()
            .map(|(re, negative, im)| {
                format!(
                    "{} {}{}i",
                    pad_left(&re, re_width),
                    if negative { '-' } else { '+' },
                    pad_left(&im, im_width)
                )
            })
            .collect();
        return Grid {
            rows,
            cols,
            width: re_width + 2 + im_width + 1,
            cells,
            scale: None,
        };
    }
    let (max_abs, _) = survey(data.iter().flat_map(|z| [z.re, z.im]));
    let (exp, scale) = if st.notation == Notation::Fixed {
        (
            data.len() == 1 && max_abs != 0. && !fits_fixed(max_abs, COMPLEX_FIXED_DIGITS, st),
            scale_of(max_abs, data.len(), COMPLEX_FIXED_DIGITS, st),
        )
    } else {
        (true, None)
    };
    let (re_width, im_width) = if exp {
        (st.complex_exp_re_width, st.decimals + 7)
    } else {
        (st.complex_re_width, st.complex_im_width)
    };
    let cells: Vec<String> = data
        .iter()
        .map(|z| {
            // The reference prints a `-0` imaginary part (and NaN) with `+`.
            let negative = z.im < 0.;
            format!(
                "{} {}{}i",
                pad_left(&complex_part(z.re, scale, exp, st), re_width),
                if negative { '-' } else { '+' },
                pad_left(&complex_part(z.im.abs(), scale, exp, st), im_width)
            )
        })
        .collect();
    Grid {
        rows,
        cols,
        width: re_width + 2 + im_width + 1,
        cells,
        scale,
    }
}

fn scale_header(exponent: i32) -> String {
    format!(
        "   1.0e{}{:02} *",
        if exponent < 0 { '-' } else { '+' },
        exponent.abs()
    )
}

/// The reference says `Columns 18 through 19` for two columns too (no `and` form).
fn columns_header(first: usize, last: usize) -> String {
    if first == last {
        format!("  Column {}", first + 1)
    } else {
        format!("  Columns {} through {}", first + 1, last + 1)
    }
}

/// Column ranges that fit the console: as many whole columns as fit, at least one.
fn column_blocks(widths: &[usize], console: usize) -> Vec<(usize, usize)> {
    let mut blocks = Vec::new();
    let (mut first, mut used) = (0, 0);
    for (col, &width) in widths.iter().enumerate() {
        if col > first && used + width > console {
            blocks.push((first, col));
            (first, used) = (col, 0);
        }
        used += width;
    }
    if first < widths.len() {
        blocks.push((first, widths.len()));
    }
    blocks
}

/// Rows of `rows`-by-`widths.len()` cells, wrapped into `Columns a through b`
/// blocks that fit the console; `cell(row, col, out)` appends one cell.
fn wrapped_rows(
    rows: usize,
    widths: &[usize],
    fmt: &DisplayFormat,
    out: &mut String,
    mut cell: impl FnMut(usize, usize, &mut String),
) -> Result<()> {
    let loose = !fmt.compact;
    let blocks = column_blocks(widths, fmt.width);
    for (block, &(first, last)) in blocks.iter().enumerate() {
        if blocks.len() > 1 {
            if block > 0 && loose {
                out.push('\n');
            }
            out.push_str(&columns_header(first, last - 1));
            out.push('\n');
            if loose {
                out.push('\n');
            }
        }
        for row in 0..rows {
            for col in first..last {
                cell(row, col, out);
            }
            out.push('\n');
            if out.len() > fmt.max_bytes {
                return Err(output_too_large(fmt));
            }
        }
    }
    Ok(())
}

/// Rows of a grid, with the scale factor line once above every block.
fn grid_body(grid: &Grid, fmt: &DisplayFormat) -> Result<String> {
    let mut out = String::new();
    if let Some(exponent) = grid.scale {
        out.push_str(&scale_header(exponent));
        out.push('\n');
        if !fmt.compact {
            out.push('\n');
        }
    }
    wrapped_rows(
        grid.rows,
        &vec![grid.width; grid.cols],
        fmt,
        &mut out,
        |row, col, out| {
            out.push_str(&grid.cells[col * grid.rows + row]);
        },
    )?;
    Ok(out)
}

fn two_d_numeric_matrix(dims: &[usize], class: &str) -> bool {
    dims.len() == 2
        && matches!(
            class,
            "double"
                | "single"
                | "int8"
                | "uint8"
                | "int16"
                | "uint16"
                | "int32"
                | "uint32"
                | "int64"
                | "uint64"
        )
}

/// Assignment caption for an empty value, without the console indent.
/// `[]` only for a 0×0 double. Every other empty uses U+00D7:
/// `0×5 empty double matrix` for a 2-D numeric class, otherwise `array`
/// (`3×0×5 empty double array`, `0×2 empty logical array`, `0×0 empty char array`).
pub fn empty_caption(dims: &[usize], class: &str) -> String {
    if class == "double" && dims == [0, 0] {
        return "[]".into();
    }
    let shape = dims
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join("\u{00d7}");
    let kind = if two_d_numeric_matrix(dims, class) {
        "matrix"
    } else {
        "array"
    };
    format!("{shape} empty {class} {kind}")
}

fn empty_echo_line(dims: &[usize], class: &str) -> String {
    let caption = empty_caption(dims, class);
    if caption == "[]" {
        "     []\n".into()
    } else {
        format!("  {caption}\n")
    }
}

fn real_body(rows: usize, cols: usize, data: &[f64], fmt: &DisplayFormat) -> Result<String> {
    if data.is_empty() {
        return Ok(empty_echo_line(&[rows, cols], "double"));
    }
    grid_body(&real_grid(rows, cols, data, &fmt.style(), fmt)?, fmt)
}

fn complex_body(m: &ComplexMatrix, fmt: &DisplayFormat) -> Result<String> {
    if m.is_empty() {
        return Ok(empty_echo_line(&[m.rows(), m.cols()], "double"));
    }
    check_size(m.len(), 20, fmt)?;
    grid_body(
        &complex_grid(m.rows(), m.cols(), m.data(), &fmt.style()),
        fmt,
    )
}

fn logical_body(m: &LogicalMatrix, fmt: &DisplayFormat) -> Result<String> {
    if m.is_empty() {
        return Ok(empty_echo_line(&[m.rows(), m.cols()], "logical"));
    }
    check_size(m.len(), 6, fmt)?;
    let data: Vec<f64> = m.data().iter().map(|&v| f64::from(v)).collect();
    // Logical entries stay 0 and 1. Engineering notation is for numeric values.
    if fmt.notation == Notation::Engineering {
        let mut plain = *fmt;
        plain.notation = Notation::Fixed;
        return real_body(m.rows(), m.cols(), &data, &plain);
    }
    real_body(m.rows(), m.cols(), &data, fmt)
}

/// One line per stored entry in column order: `(r,c)` right-aligned to the
/// widest index pair, then the value as the full display formats the column
/// of stored values in the current format.
fn sparse_body(s: &crate::sparse::SparseMatrix, fmt: &DisplayFormat) -> Result<String> {
    if s.nnz() == 0 {
        return Ok(format!("   All zero sparse: {}x{}\n", s.rows(), s.cols()));
    }
    check_size(s.nnz(), 24, fmt)?;
    let values: Vec<f64> = (0..s.nnz()).map(|k| s.value(k)).collect();
    let column = real_body(values.len(), 1, &values, fmt)?;
    let lines: Vec<&str> = column.lines().collect();
    // The value lines are the last nnz non-blank lines; anything before them
    // (a common scale factor) is kept as a heading.
    let mut value_lines: Vec<&str> = Vec::with_capacity(values.len());
    let mut cut = lines.len();
    while cut > 0 && value_lines.len() < values.len() {
        cut -= 1;
        if !lines[cut].trim().is_empty() {
            value_lines.push(lines[cut]);
        }
    }
    value_lines.reverse();
    let pairs: Vec<String> = s
        .entries()
        .map(|(r, c, _)| format!("({},{})", r + 1, c + 1))
        .collect();
    let width = pairs.iter().map(String::len).max().unwrap_or(0);
    let mut out = String::new();
    for line in &lines[..cut] {
        out.push_str(line);
        out.push('\n');
    }
    for (pair, line) in pairs.iter().zip(&value_lines) {
        out.push_str("   ");
        out.push_str(&format!("{pair:>width$}"));
        out.push_str(line);
        out.push('\n');
        if out.len() > fmt.max_bytes {
            return Err(output_too_large(fmt));
        }
    }
    Ok(out)
}

fn decimal_digits(mut n: u128) -> usize {
    let mut digits = 1;
    while n >= 10 {
        n /= 10;
        digits += 1;
    }
    digits
}

/// Column width for integer display: scalars use 3 leading spaces (matching
/// the public MathWorks `int32`/`uint64` pages); arrays use Octave's
/// `max_digits + (1 if any negative) + 2` column width (matching Octave 10.3's
/// `pr_int`, which ignores signed `T::MIN` in `max_digits`/`has_neg`).
fn integer_display_width(a: &TypedArray) -> usize {
    if a.len() <= 1 {
        return a.int(0).to_string().len() + 3;
    }
    let signed_min = match a.class() {
        NumericClass::Int8 => Some(i128::from(i8::MIN)),
        NumericClass::Int16 => Some(i128::from(i16::MIN)),
        NumericClass::Int32 => Some(i128::from(i32::MIN)),
        NumericClass::Int64 => Some(i128::from(i64::MIN)),
        _ => None,
    };
    let mut max_digits = 1usize;
    let mut has_neg = false;
    for i in 0..a.len() {
        let v = a.int(i);
        if Some(v) == signed_min {
            continue;
        }
        if v < 0 {
            has_neg = true;
        }
        max_digits = max_digits.max(decimal_digits(v.unsigned_abs()));
    }
    max_digits + usize::from(has_neg) + 2
}

/// Integer scalars print with 3 leading spaces (data-width + 3) and arrays
/// use Octave's data-width column spacing; variable echoes prepend the class
/// label line (`  int32`, etc.). Single prints like double, with 7 decimals
/// under `format long`.
fn typed_body(
    a: &TypedArray,
    fmt: &DisplayFormat,
    header: bool,
    width_override: Option<usize>,
) -> Result<String> {
    if a.is_empty() {
        return Ok(empty_echo_line(a.dims().as_ref(), a.class().name()));
    }
    if !a.is_integer() {
        let st = fmt.single_style();
        let data = a.to_f64_vec()?;
        return grid_body(&real_grid(a.rows(), a.cols(), &data, &st, fmt)?, fmt);
    }
    let scalar_mode = width_override.is_none() && a.len() <= 1;
    let width = width_override.unwrap_or_else(|| integer_display_width(a));
    let rows = a.rows();
    let cols = a.cols();
    // Reject oversized output (including newlines) before allocating.
    check_size(rows, cols.saturating_mul(width).saturating_add(1), fmt)?;
    let mut out = String::new();
    if header {
        out.push_str(&format!("  {}\n", a.class().name()));
        if !fmt.compact {
            out.push('\n');
        }
    }
    wrapped_rows(rows, &vec![width; cols], fmt, &mut out, |row, col, out| {
        let v = a.int(col * rows + row);
        if scalar_mode {
            let _ = write!(out, "{v:>width$}");
        } else {
            let mut buf = [0u8; 48];
            let mut cursor = std::io::Cursor::new(&mut buf[..]);
            let _ = std::io::Write::write_fmt(&mut cursor, format_args!("{v}"));
            let len = cursor.position() as usize;
            let text = std::str::from_utf8(&buf[..len]).unwrap_or("");
            let lead = width.saturating_sub(2).saturating_sub(len) + 2;
            out.extend(std::iter::repeat_n(' ', lead));
            out.push_str(text);
        }
    })?;
    Ok(out)
}

fn text_of(units: &[u16]) -> String {
    let mut s = String::from_utf16_lossy(&units[..units.len().min(MAX_TEXT_CHARS)]);
    if units.len() > MAX_TEXT_CHARS {
        s.push_str("...");
    }
    s
}

fn char_body(c: &CharacterRow, fmt: &DisplayFormat) -> String {
    if c.is_empty() {
        let mut body = empty_echo_line(c.dims().as_ref(), "char");
        // The reference leaves an extra blank line after an empty char in loose format.
        if !fmt.compact {
            body.push('\n');
        }
        return body;
    }
    let (rows, cols) = (c.rows(), c.cols());
    let mut out = String::new();
    for r in 0..rows {
        let line: Vec<u16> = (0..cols).map(|col| c.units()[col * rows + r]).collect();
        out.push_str(&String::from_utf16_lossy(&line));
        out.push('\n');
    }
    out
}

/// What a cell shows for one element.
enum Summary {
    /// A scalar, shown as `[value]` and right-aligned inside the brackets to the
    /// column width (`[         1]` above `[1x2 double]`).
    Value(String),
    /// Anything else (`'two'`, `[1x2 double]`, `{1x2 cell}`), left-aligned.
    Text(String),
}

impl Summary {
    fn width(&self) -> usize {
        match self {
            Summary::Value(v) => v.chars().count() + 2,
            Summary::Text(t) => t.chars().count(),
        }
    }

    /// `pad`: fill text to the column width (every column of a cell with
    /// several columns; the reference does not pad a single-column cell).
    fn push(&self, width: usize, pad: bool, out: &mut String) {
        match self {
            Summary::Value(v) => out.push_str(&format!("[{v:>0$}]", width - 2)),
            Summary::Text(t) if pad => out.push_str(&format!("{t:<width$}")),
            Summary::Text(t) => out.push_str(t),
        }
    }
}

fn cell_summary(a: &Array, st: &Style) -> Summary {
    let dims = format!("{}x{}", a.rows(), a.cols());
    Summary::Text(match a {
        Array::Numeric(m) if m.len() == 1 => return Summary::Value(scalar_text(m.data()[0], st)),
        Array::Logical(m) if m.len() == 1 => return Summary::Value(m.data()[0].to_string()),
        Array::Typed(t) if t.len() == 1 && t.is_integer() => {
            return Summary::Value(t.int(0).to_string());
        }
        Array::Typed(t) if t.len() == 1 => return Summary::Value(scalar_text(t.real(0), st)),
        Array::Complex(m) if m.len() == 1 => {
            return Summary::Value(complex_scalar_text(m.data()[0], st));
        }
        Array::Numeric(_) | Array::Logical(_) if a.rows() == 0 && a.cols() == 0 => "[]".into(),
        Array::Str(s) if s.len() == 1 => s.preview(1),
        Array::Character(c) if c.is_empty() => "''".into(),
        Array::Character(c) if c.rows() == 1 => {
            format!("'{}'", text_of(c.units()).replace('\'', "''"))
        }
        Array::Cell(c) if c.rows() == 0 && c.cols() == 0 => "{}".into(),
        Array::Cell(_) => format!("{{{dims} cell}}"),
        _ => format!("[{dims} {}]", a.class()),
    })
}

/// A row cell as a struct field shows it, elements two spaces apart:
/// `{[1]  [2]}`, `{''}`, `{[]}` and `{}` for a 0x0 cell (Q1, Q5). `None` for a
/// cell of several rows or more than `max_elements` elements.
pub(crate) fn cell_inline(c: &CellArray, max_elements: usize, st: &Style) -> Option<String> {
    if c.rows() == 0 && c.cols() == 0 {
        return Some("{}".into());
    }
    if c.rows() != 1 || c.len() > max_elements {
        return None;
    }
    let mut parts = Vec::with_capacity(c.len());
    for i in 0..c.len() {
        parts.push(match cell_summary(c.get_linear(i).ok()?, st) {
            Summary::Value(v) => format!("[{v}]"),
            Summary::Text(t) => t,
        });
    }
    Some(format!("{{{}}}", parts.join("  ")))
}

/// A 1x1 cell whose element is `[]`, `''` or `{}` shows the cell braces too:
/// the reference prints `{''}` and `{{}}` for `x = {''}` and `x = {{}}`.
fn braced_empty(a: &Array) -> bool {
    a.rows() == 0
        && a.cols() == 0
        && matches!(
            a,
            Array::Numeric(_) | Array::Logical(_) | Array::Character(_) | Array::Cell(_)
        )
}

/// Mark that ends a shortened character row inside a cell row (U+2026; the
/// reference corpora saved it through `evalc` as the substitute byte 0x1A).
pub(crate) const CELL_ELLIPSIS: char = '\u{2026}';

/// Columns a quoted character row may take inside a cell with `cols` columns
/// (its four leading spaces included); `None` for a single-column cell, which
/// shows text whole. Rule fitted to every cell row of the reference corpora in
/// `tests/fixtures` (tool/refs/followups/cell_ellipsis_scan.py): the console
/// width shared by at most four columns, so 107 gives 53, 35 and then 26.
fn cell_text_budget(cols: usize, width: usize) -> Option<usize> {
    (cols > 1).then(|| width / cols.min(4))
}

/// `'text'` shortened to `budget - 8` characters and the ellipsis when the
/// quoted row and its column gap exceed `budget` (reference: 1x3 cells keep
/// 27 characters of `'MATLAB:RMDIR:NoDirectoriesRemoved'`, 1x4 cells 18).
/// A narrow console (budget under 9) keeps one character. The text stays
/// whole unless the cut makes it narrower, so the mark never stands alone,
/// never replaces a single character and never makes the row wider.
fn shorten_cell_text(quoted: String, budget: Option<usize>) -> String {
    let Some(budget) = budget else {
        return quoted;
    };
    let Some(inner) = quoted
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
    else {
        return quoted;
    };
    if inner.chars().count() + 6 <= budget {
        return quoted;
    }
    let chars: Vec<char> = inner.chars().collect();
    let mut keep = budget.saturating_sub(8).max(1).min(chars.len());
    // Never split a doubled quote (`''` shows one apostrophe): an odd run of
    // quotes at the cut loses its unpaired half, or keeps the pair when that
    // half is the only character shown.
    let quotes = chars[..keep].iter().rev().take_while(|&&c| c == '\'');
    if quotes.count() % 2 == 1 {
        keep = if keep == 1 { 2 } else { keep - 1 };
    }
    if keep + 1 >= chars.len() {
        return quoted;
    }
    let shown: String = chars[..keep].iter().collect();
    format!("'{shown}{CELL_ELLIPSIS}'")
}

fn cell_body(c: &CellArray, fmt: &DisplayFormat, stand_ins: &[(String, String)]) -> Result<String> {
    let st = fmt.style();
    let (rows, cols) = (c.rows(), c.cols());
    if c.is_empty() {
        return Ok(if rows == 0 && cols == 0 {
            "     {}\n".into()
        } else {
            format!("   Empty cell array: {rows}-by-{cols}\n")
        });
    }
    let mut summaries = Vec::with_capacity(c.len());
    let mut bytes = 0usize;
    let budget = cell_text_budget(cols, fmt.width);
    for i in 0..c.len() {
        let element = c.get_linear(i)?;
        let summary = match cell_summary(element, &st) {
            Summary::Text(t) if c.len() == 1 && braced_empty(element) => {
                Summary::Text(format!("{{{t}}}"))
            }
            Summary::Text(t) if matches!(&**element, Array::Character(_)) => {
                let text = match stand_ins.iter().find(|(marker, _)| marker == &t) {
                    Some((_, label)) => label.clone(),
                    None => shorten_cell_text(t, budget),
                };
                Summary::Text(text)
            }
            summary => summary,
        };
        bytes += summary.width() + 4;
        if bytes > fmt.max_bytes {
            return Err(output_too_large(fmt));
        }
        summaries.push(summary);
    }
    let widths: Vec<usize> = (0..cols)
        .map(|col| {
            (0..rows)
                .map(|r| summaries[col * rows + r].width())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let spaced: Vec<usize> = widths.iter().map(|w| w + 4).collect();
    let mut out = String::new();
    wrapped_rows(rows, &spaced, fmt, &mut out, |row, col, out| {
        out.push_str("    ");
        summaries[col * rows + row].push(widths[col], cols > 1, out);
    })?;
    Ok(out)
}

/// Display a cell with object labels substituted before sizing its columns.
pub(crate) fn echo_cell_with_stand_ins(
    name: &str,
    cell: &CellArray,
    fmt: &DisplayFormat,
    stand_ins: &[(String, String)],
) -> Result<String> {
    checked(
        wrap(name, true, &cell_body(cell, fmt, stand_ins)?, fmt),
        fmt,
    )
}

/// Header, blank lines and the body as MATLAB prints `name = value`, minus the
/// final newline.
pub(crate) fn wrap(name: &str, spaced_equals: bool, body: &str, fmt: &DisplayFormat) -> String {
    let mut full = String::with_capacity(body.len() + name.len() + 8);
    if !fmt.compact {
        full.push('\n');
    }
    full.push_str(name);
    full.push_str(if spaced_equals { " = \n" } else { " =\n" });
    if !fmt.compact {
        full.push('\n');
    }
    full.push_str(body);
    if !fmt.compact {
        full.push('\n');
    }
    full.truncate(full.len() - 1);
    full
}

pub fn echo_real(name: &str, m: &Matrix, fmt: &DisplayFormat) -> Result<String> {
    let body = real_body(m.rows(), m.cols(), m.data(), fmt)?;
    checked(wrap(name, false, &body, fmt), fmt)
}

pub fn echo_logical(name: &str, m: &LogicalMatrix, fmt: &DisplayFormat) -> Result<String> {
    checked(wrap(name, false, &logical_body(m, fmt)?, fmt), fmt)
}

pub fn echo_complex(name: &str, m: &ComplexMatrix, fmt: &DisplayFormat) -> Result<String> {
    checked(wrap(name, false, &complex_body(m, fmt)?, fmt), fmt)
}

/// Text of an unsuppressed `name = value` statement.
pub fn echo(name: &str, array: &Array, fmt: &DisplayFormat) -> Result<String> {
    if array.is_nd() {
        if array.is_empty() {
            return checked(
                wrap(
                    name,
                    false,
                    &empty_echo_line(array.dims().as_ref(), array.class()),
                    fmt,
                ),
                fmt,
            );
        }
        return nd_pages(Some(name), array, fmt);
    }
    let text = match array {
        Array::Numeric(m) => echo_real(name, m, fmt),
        Array::Logical(m) => echo_logical(name, m, fmt),
        Array::Complex(m) => echo_complex(name, m, fmt),
        Array::Character(c) => Ok(wrap(name, false, &char_body(c, fmt), fmt)),
        Array::Str(s) => Ok(wrap(name, false, &str_body(s, fmt, true)?, fmt)),
        Array::Cell(c) => return echo_cell_with_stand_ins(name, c, fmt, &[]),
        Array::Struct(s) => Ok(crate::struct_display::named(name, s, fmt)),
        Array::Graphics(g) => Ok(wrap(name, false, &format!("  {g}\n"), fmt)),
        Array::Handle(handle) => Ok(format!("{name} = \n\n    {}\n", handle.display())),
        Array::Typed(a) => Ok(wrap(name, false, &typed_body(a, fmt, true, None)?, fmt)),
        Array::Categorical(c) => {
            let rendered = wrap(name, true, &categorical_body(c, fmt, true)?, fmt);
            Ok(rendered.strip_prefix('\n').unwrap_or(&rendered).to_owned())
        }
        Array::Datetime(_) | Array::Duration(_) => {
            Ok(wrap(name, false, &time_body(array, fmt, true)?, fmt))
        }
        Array::Table(t) => Ok(wrap(name, false, &table_body(t, fmt)?, fmt)),
        Array::Sparse(s) => Ok(wrap(name, false, &sparse_body(s, fmt)?, fmt)),
    }?;
    checked(text, fmt)
}

/// Text of `disp(value)`; `None` when MATLAB prints nothing (any empty value,
/// a 0x0 struct array too: Q7).
pub fn disp(array: &Array, fmt: &DisplayFormat) -> Result<Option<String>> {
    if array.is_nd() {
        if array.is_empty() {
            return Ok(None);
        }
        return Ok(Some(nd_pages(None, array, fmt)?.trim_end().to_owned()));
    }
    let (body, blank_after) = match array {
        Array::Table(t) => (table_body(t, fmt)?, true),
        a if a.is_empty() => return Ok(None),
        Array::Struct(s) => return checked(crate::struct_display::disp(s, fmt), fmt).map(Some),
        Array::Handle(handle) => {
            return checked(format!("    {}", handle.display()), fmt).map(Some);
        }
        Array::Numeric(m) => (real_body(m.rows(), m.cols(), m.data(), fmt)?, true),
        Array::Logical(m) => (logical_body(m, fmt)?, true),
        Array::Complex(m) => (complex_body(m, fmt)?, true),
        Array::Typed(a) => (typed_body(a, fmt, false, None)?, true),
        Array::Categorical(c) => (categorical_body(c, fmt, false)?, false),
        Array::Datetime(_) | Array::Duration(_) => (time_body(array, fmt, false)?, false),
        Array::Character(c) => (char_body(c, fmt), false),
        Array::Str(s) => (str_body(s, fmt, false)?, false),
        Array::Cell(c) => (cell_body(c, fmt, &[])?, true),
        Array::Graphics(g) => (format!("{g}\n"), false),
        Array::Sparse(s) => (sparse_body(s, fmt)?, true),
    };
    let mut full = body;
    if blank_after && !fmt.compact {
        full.push('\n');
    }
    full.truncate(full.len() - 1);
    checked(full, fmt).map(Some)
}

fn nd_pages(name: Option<&str>, array: &Array, fmt: &DisplayFormat) -> Result<String> {
    let dims = array.dims();
    let rows = dims[0];
    let cols = dims[1];
    let page_len = rows
        .checked_mul(cols)
        .ok_or("Display dimensions overflow")?;
    let pages = dims[2..]
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .ok_or("Display dimensions overflow")?;
    let mut output = if name.is_some() && !fmt.compact {
        String::from("\n")
    } else {
        String::new()
    };
    let int_width = match array {
        Array::Typed(a) if a.is_integer() => Some(integer_display_width(a)),
        _ => None,
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    for page in 0..pages {
        check_size(output.len(), 1, fmt)?;
        if page > 0 {
            output.push_str(if fmt.compact { "\n" } else { "\n\n" });
        }
        let label = match name {
            Some(name) => format!("{name}(:,:,{}) =\n", page + 1),
            None => format!("(:,:,{}) =\n", page + 1),
        };
        output.push_str(&label);
        if !fmt.compact {
            output.push('\n');
        }
        let start = page * page_len;
        let end = start + page_len;
        let body = match array {
            Array::Numeric(a) => real_body(rows, cols, &a.data()[start..end], fmt)?,
            Array::Logical(a) => {
                let part = LogicalMatrix::from_column_vec(
                    rows,
                    cols,
                    a.data()[start..end].to_vec(),
                    &cancel,
                )?;
                logical_body(&part, fmt)?
            }
            Array::Complex(a) => {
                let part = ComplexMatrix::from_column_vec(
                    rows,
                    cols,
                    a.data()[start..end].to_vec(),
                    &cancel,
                )?;
                complex_body(&part, fmt)?
            }
            Array::Typed(a) => {
                let positions = (start..end).collect::<Vec<_>>();
                typed_body(
                    &a.select_with_cancel(rows, cols, &positions, &cancel)?,
                    fmt,
                    name.is_some(),
                    int_width,
                )?
            }
            Array::Character(a) => {
                let part = CharacterRow::from_column_major(
                    rows,
                    cols,
                    a.units()[start..end].to_vec(),
                    &cancel,
                )?;
                char_body(&part, fmt)
            }
            _ => return Err("N-D display is not supported for this array class".into()),
        };
        output.push_str(&body);
        if !output.ends_with('\n') {
            output.push('\n');
        }
    }
    if fmt.compact && output.ends_with('\n') {
        output.pop();
    }
    checked(output, fmt)
}

// String entries are quoted individually; missing is an unquoted sentinel.
/// datetime and duration arrays: a class line (echo only), then the
/// elements in their display format, right-aligned in each column.
fn time_body(array: &Array, fmt: &DisplayFormat, header: bool) -> Result<String> {
    let (rows, cols) = (array.rows(), array.cols());
    let class = array.class();
    let mut body = String::new();
    if header {
        if array.is_empty() {
            body.push_str(&format!("  {rows}x{cols} empty {class} array\n"));
            return Ok(body);
        }
        if rows * cols == 1 {
            body.push_str(&format!("  {class}\n"));
        } else {
            body.push_str(&format!("  {rows}x{cols} {class} array\n"));
        }
        if !fmt.compact {
            body.push('\n');
        }
    }
    if array.is_empty() {
        return Ok(body);
    }
    check_size(array.len(), 8, fmt)?;
    let cells = crate::engine::datetime_fns::display::element_texts(array, None, None)?;
    let mut widths = vec![0; cols];
    let mut bytes = body.len();
    for (i, text) in cells.iter().enumerate() {
        let c = i / rows;
        widths[c] = widths[c].max(text.chars().count());
        bytes = bytes.saturating_add(text.len() + 3);
    }
    check_size(bytes, 1, fmt)?;
    let spaced: Vec<usize> = widths.iter().map(|w| w + 3).collect();
    wrapped_rows(rows, &spaced, fmt, &mut body, |r, c, out| {
        let text = &cells[c * rows + r];
        out.extend(std::iter::repeat_n(
            ' ',
            3 + widths[c] - text.chars().count(),
        ));
        out.push_str(text);
    })?;
    Ok(body)
}

fn str_body(s: &crate::StringArray, fmt: &DisplayFormat, quoted: bool) -> Result<String> {
    let show_elements = quoted || s.len() != 1;
    let mut body = String::new();
    if s.is_empty() {
        if quoted {
            body.push_str(&empty_echo_line(&[s.rows(), s.cols()], "string"));
        }
        return Ok(body);
    }
    if quoted && s.len() != 1 {
        body.push_str(&format!("  {}\u{00d7}{} string\n", s.rows(), s.cols()));
        if !fmt.compact {
            body.push('\n');
        }
    }
    let mut cells = Vec::with_capacity(s.len());
    let mut widths = vec![0; s.cols()];
    let mut bytes = body.len();
    for (i, item) in s.data().iter().enumerate() {
        let text = if show_elements {
            crate::StringArray::quoted(item)
        } else {
            item.as_deref().unwrap_or("<missing>").to_owned()
        };
        bytes = bytes.saturating_add(text.len()).saturating_add(4);
        if bytes > fmt.max_bytes {
            return Err("String display exceeds output limit".into());
        }
        widths[i / s.rows()] = widths[i / s.rows()].max(text.chars().count());
        cells.push(text);
    }
    let mut exact_bytes = body.len() + s.rows() * 5;
    for (i, text) in cells.iter().enumerate() {
        let c = i / s.rows();
        exact_bytes += text.len();
        if c + 1 < s.cols() {
            exact_bytes += widths[c] - text.chars().count() + 4;
        }
    }
    if exact_bytes > fmt.max_bytes {
        return Err("String display exceeds output limit".into());
    }
    for r in 0..s.rows() {
        if show_elements {
            body.push_str("    ");
        }
        for (c, &width) in widths.iter().enumerate() {
            let text = &cells[c * s.rows() + r];
            body.push_str(text);
            if c + 1 < s.cols() {
                body.extend(std::iter::repeat_n(' ', width - text.chars().count() + 4));
            }
        }
        body.push('\n');
    }
    Ok(body)
}

/// One displayed sub-column of a table variable: its texts (one per row) and
/// whether they are right-aligned (numbers, logicals).
struct TableColumn {
    texts: Vec<String>,
    right: bool,
}

/// A variable as displayed: header, column width, sub-columns with widths.
type TableGroup = (String, usize, Vec<(TableColumn, usize)>);

/// The sub-columns a table variable shows, one per column of the variable.
fn table_columns(a: &Array, st: &Style) -> Result<Vec<TableColumn>> {
    let (rows, cols) = (a.rows(), a.cols());
    let per = |right: bool, f: &dyn Fn(usize) -> String| -> Vec<TableColumn> {
        (0..cols)
            .map(|c| TableColumn {
                texts: (0..rows).map(|r| f(c * rows + r)).collect(),
                right,
            })
            .collect()
    };
    Ok(match a {
        Array::Numeric(m) => per(true, &|i| scalar_text(m.data()[i], st)),
        Array::Logical(m) => per(true, &|i| {
            if m.data()[i] != 0 { "true" } else { "false" }.to_owned()
        }),
        Array::Complex(m) => per(true, &|i| complex_scalar_text(m.data()[i], st)),
        Array::Typed(t) if t.is_integer() => per(true, &|i| t.int(i).to_string()),
        Array::Typed(t) => per(true, &|i| scalar_text(t.real(i), st)),
        // Full text, quoted as a string array shows it (no preview cut).
        Array::Str(s) => per(false, &|i| crate::StringArray::quoted(&s.data()[i])),
        Array::Cell(c) => {
            let mut out = Vec::with_capacity(cols);
            for col in 0..cols {
                let mut texts = Vec::with_capacity(rows);
                for r in 0..rows {
                    let item = c.get_linear(col * rows + r)?;
                    texts.push(match cell_summary(item, st) {
                        Summary::Value(v) => format!("{{[{v}]}}"),
                        Summary::Text(t) => format!("{{{t}}}"),
                    });
                }
                out.push(TableColumn {
                    texts,
                    right: false,
                });
            }
            out
        }
        Array::Character(c) => {
            // Each row of a char matrix is one text, shown unquoted.
            let texts = (0..rows)
                .map(|r| {
                    let row: Vec<u16> = (0..cols).map(|k| c.units()[k * rows + r]).collect();
                    text_of(&row)
                })
                .collect();
            vec![TableColumn {
                texts,
                right: false,
            }]
        }
        Array::Categorical(c) => per(false, &|i| c.text(i).to_string()),
        Array::Datetime(_) | Array::Duration(_) => {
            let cells = crate::engine::datetime_fns::display::element_texts(a, None, None)?;
            per(false, &|i| cells[i].clone())
        }
        other => {
            let class = other.class();
            vec![TableColumn {
                texts: (0..rows).map(|_| format!("[1x{cols} {class}]")).collect(),
                right: false,
            }]
        }
    })
}

/// A table as `T = ...` and `disp(T)` show it: a size line, then each
/// variable under its name and a rule of underscores as wide as the column,
/// row names (if any) as a first column without a header. Columns are four
/// spaces apart; numbers are right-aligned, text left-aligned.
fn table_body(t: &crate::TableArray, fmt: &DisplayFormat) -> Result<String> {
    let (h, w) = (t.height(), t.width());
    if h == 0 || (w == 0 && !t.is_timetable()) {
        return Ok(format!(
            "  {h}x{w} empty {}\n",
            if t.is_timetable() {
                "timetable"
            } else {
                "table"
            }
        ));
    }
    check_size(
        t.len().saturating_add(if t.is_timetable() { h } else { 0 }),
        8,
        fmt,
    )?;
    let st = fmt.style();
    const SEP: &str = "    ";
    // (header, width, sub-columns with their widths)
    let mut groups: Vec<TableGroup> = Vec::with_capacity(w);
    let mut bytes = 0usize;
    let times = t.row_times().map(|a| (&t.parts().dimension_names[0], a));
    for (name, column) in times.into_iter().chain(t.names().iter().zip(t.columns())) {
        let subs: Vec<(TableColumn, usize)> = table_columns(column, &st)?
            .into_iter()
            .map(|c| {
                let width = c.texts.iter().map(|s| s.chars().count()).max().unwrap_or(0);
                (c, width)
            })
            .collect();
        let inner =
            subs.iter().map(|(_, w)| *w).sum::<usize>() + SEP.len() * subs.len().saturating_sub(1);
        let width = inner.max(name.chars().count());
        bytes = bytes.saturating_add((width + SEP.len()).saturating_mul(h + 2));
        check_size(bytes, 1, fmt)?;
        groups.push((name.clone(), width, subs));
    }
    let row_names = t.row_names();
    let label_width = row_names.map_or(0, |n| {
        n.iter().map(|s| s.chars().count()).max().unwrap_or(0)
    });
    let mut body = format!(
        "  {h}x{w} {}\n",
        if t.is_timetable() {
            "timetable"
        } else {
            "table"
        }
    );
    if !fmt.compact {
        body.push('\n');
    }
    let lead = |line: &mut String| {
        line.push_str(SEP);
        if row_names.is_some() {
            line.extend(std::iter::repeat_n(' ', label_width));
            line.push_str(SEP);
        }
    };
    let finish = |line: String, body: &mut String| {
        body.push_str(line.trim_end());
        body.push('\n');
    };
    let mut header = String::new();
    let mut rule = String::new();
    lead(&mut header);
    lead(&mut rule);
    for (k, (name, width, _)) in groups.iter().enumerate() {
        if k > 0 {
            header.push_str(SEP);
            rule.push_str(SEP);
        }
        header.push_str(&format!("{name:<width$}"));
        rule.extend(std::iter::repeat_n('_', *width));
    }
    finish(header, &mut body);
    finish(rule, &mut body);
    if !fmt.compact {
        body.push('\n');
    }
    for r in 0..h {
        let mut line = String::from(SEP);
        if let Some(names) = row_names {
            line.push_str(&format!("{:<label_width$}", names[r]));
            line.push_str(SEP);
        }
        for (k, (_, width, subs)) in groups.iter().enumerate() {
            if k > 0 {
                line.push_str(SEP);
            }
            let start = line.chars().count();
            for (j, (sub, sw)) in subs.iter().enumerate() {
                if j > 0 {
                    line.push_str(SEP);
                }
                // A lone sub-column fills the whole column (under a longer name).
                let sw = if subs.len() == 1 { *width } else { *sw };
                let text = &sub.texts[r];
                if sub.right {
                    line.push_str(&format!("{text:>sw$}"));
                } else {
                    line.push_str(&format!("{text:<sw$}"));
                }
            }
            let used = line.chars().count() - start;
            line.extend(std::iter::repeat_n(' ', width.saturating_sub(used)));
        }
        finish(line, &mut body);
    }
    check_size(body.len(), 1, fmt)?;
    Ok(body)
}

/// Categorical values are unquoted, left aligned, and retain a trailing space.
fn categorical_body(
    c: &crate::CategoricalArray,
    fmt: &DisplayFormat,
    header: bool,
) -> Result<String> {
    let mut body = String::new();
    if header {
        if c.is_empty() {
            return Ok(format!(
                "  {}x{} empty categorical array\n",
                c.rows(),
                c.cols()
            ));
        }
        if c.len() == 1 {
            body.push_str("  categorical\n");
        } else {
            body.push_str(&format!("  {}x{} categorical array\n", c.rows(), c.cols()));
        }
        if !fmt.compact {
            body.push('\n');
        }
    }
    check_size(c.len(), 6, fmt)?;
    let mut widths = vec![0; c.cols()];
    let mut bytes = body.len();
    for i in 0..c.len() {
        widths[i / c.rows()] = widths[i / c.rows()].max(c.text(i).chars().count());
        bytes = bytes.saturating_add(c.text(i).len() + 6);
    }
    check_size(bytes, 1, fmt)?;
    let spaced: Vec<usize> = widths.iter().map(|w| w + 6).collect();
    wrapped_rows(c.rows(), &spaced, fmt, &mut body, |r, col, out| {
        let text = c.text(col * c.rows() + r);
        out.push_str("     ");
        out.push_str(text);
        out.push(' ');
        if col + 1 < c.cols() {
            out.extend(std::iter::repeat_n(' ', widths[col] - text.chars().count()));
        }
    })?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::general_form;
    use std::f64::consts::{E, PI};

    /// `%.5g`, `%.15g` and `%.7g` of each value from CPython 3 (C printf rules):
    /// `python3 -c "print('%.5g %.15g %.7g' % (v, v, v))"`. No value is a
    /// rounding tie, where printf (exact binary value) and the reference
    /// (17 significant digits, half up) may differ.
    const CASES: &[(f64, [&str; 3])] = &[
        (PI, ["3.1416", "3.14159265358979", "3.141593"]),
        (-PI, ["-3.1416", "-3.14159265358979", "-3.141593"]),
        (0.1 + 0.2, ["0.3", "0.3", "0.3"]),
        (1000.5, ["1000.5", "1000.5", "1000.5"]),
        (1e-5, ["1e-05", "1e-05", "1e-05"]),
        (0.0001, ["0.0001", "0.0001", "0.0001"]),
        (
            0.00012345678,
            ["0.00012346", "0.00012345678", "0.0001234568"],
        ),
        (123456.7, ["1.2346e+05", "123456.7", "123456.7"]),
        (99999.4, ["99999", "99999.4", "99999.4"]),
        (9.99996, ["10", "9.99996", "9.99996"]),
        (
            12345.678901234567,
            ["12346", "12345.6789012346", "12345.68"],
        ),
        (1.5e-7, ["1.5e-07", "1.5e-07", "1.5e-07"]),
        (1e20, ["1e+20", "1e+20", "1e+20"]),
        (-E, ["-2.7183", "-2.71828182845905", "-2.718282"]),
        (0.5, ["0.5", "0.5", "0.5"]),
        (1e-300, ["1e-300", "1e-300", "1e-300"]),
        (
            6.02214076e23,
            ["6.0221e+23", "6.02214076e+23", "6.022141e+23"],
        ),
        (0.001, ["0.001", "0.001", "0.001"]),
        (123456789.5, ["1.2346e+08", "123456789.5", "1.234568e+08"]),
        (0.010000000000000002, ["0.01", "0.01", "0.01"]),
    ];

    #[test]
    fn general_form_matches_printf_g() {
        for &(v, expected) in CASES {
            for (digits, text) in [5, 15, 7].into_iter().zip(expected) {
                assert_eq!(general_form(v, digits), text, "%.{digits}g of {v:e}");
            }
        }
        assert_eq!(general_form(0., 5), "0");
        assert_eq!(general_form(-0., 15), "0");
    }
}
