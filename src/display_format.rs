//! Command-window display: variable echo (`x = ...`) and `disp`.
//!
//! The reference fixtures in `tests/fixtures/display/` cover 96 initial cases,
//! 207 first-probe cases and a second probe batch. `tests/display_format.rs`
//! replays them; behavior beyond the covered cases remains a hypothesis.
//!
//! Every function returns an *entry* without its last newline, because the
//! console adds one newline after each entry.
use crate::{
    Array, CellArray, CharacterRow, Complex64, ComplexMatrix, LogicalMatrix, Matrix, Result,
    TypedArray,
};

/// Console width in characters when the host does not report one: the reference's
/// `CommandWindowSize` was 107 (P154; `1:20` splits after 17 six-wide columns).
pub const DEFAULT_WIDTH: usize = 107;
/// Same limit as `Engine::emit`; formatting stops before building more text.
const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const OUTPUT_TOO_LARGE: &str =
    "Execution output exceeds 2000 entries or 2 MiB; use semicolons to suppress output";
/// Longest text shown for one character row inside a cell or struct summary.
pub(crate) const MAX_TEXT_CHARS: usize = 1024;

/// Session display settings: `format short|long`, `format compact|loose` and
/// the console width in characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayFormat {
    pub long: bool,
    pub compact: bool,
    pub width: usize,
}

impl Default for DisplayFormat {
    fn default() -> Self {
        Self {
            long: false,
            compact: false,
            width: DEFAULT_WIDTH,
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
};

impl DisplayFormat {
    pub(crate) fn style(&self) -> Style {
        if self.long { LONG } else { SHORT }
    }

    /// Apply `format`'s option words; no option resets to `short` and `loose`.
    pub fn apply_option(&mut self, option: Option<&str>) -> Result<()> {
        match option {
            None => {
                self.long = false;
                self.compact = false;
            }
            Some("short") => self.long = false,
            Some("long") => self.long = true,
            Some("compact") => self.compact = true,
            Some("loose") => self.compact = false,
            Some(other) => {
                return Err(format!(
                    "format: unsupported option '{other}'; use short, long, compact or loose"
                ));
            }
        }
        Ok(())
    }
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
    if !v.is_finite() || is_whole(v) {
        Single::Whole
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

fn check_size(count: usize, width: usize) -> Result<()> {
    if count.saturating_mul(width) > MAX_OUTPUT_BYTES {
        Err(OUTPUT_TOO_LARGE.into())
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

fn real_grid(rows: usize, cols: usize, data: &[f64], st: &Style) -> Result<Grid> {
    let (max_abs, whole) = survey(data.iter().copied());
    let d = digits(max_abs);
    if data.len() == 1 {
        let v = data[0];
        let (text, width) = match single_kind(v, st) {
            Single::Whole => (scalar_text(v, st), if d <= 3 { 6 } else { 12 }),
            Single::Fixed => (scalar_text(v, st), st.fixed_width),
            Single::Exp => (scalar_text(v, st), st.exp_width),
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
        check_size(data.len(), width)?;
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
    let scale = scale_of(max_abs, data.len(), st.fixed_digits, st);
    check_size(data.len(), st.fixed_width)?;
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

/// One part of a complex element: never a bare `0` (`0.0000 + 1.0000i`).
fn complex_part(v: f64, scale: Option<i32>, exp: bool, st: &Style) -> String {
    if let Some(text) = non_finite(v) {
        return text.into();
    }
    // `-0` prints as `0.0000`, as in a real matrix.
    let v = if v == 0. { 0. } else { v };
    if exp {
        exp_form(v, st.decimals)
    } else {
        fixed(scale_value(v, scale), st.decimals)
    }
}

/// Complex elements `re +im i` in fixed point; a lone value (scalar echo, cell
/// and struct summaries) that fixed point cannot show prints both parts in `e`
/// form (`1.0050e+02 + 2.0000e+00i`), a matrix takes a common factor instead.
fn complex_grid(rows: usize, cols: usize, data: &[Complex64], st: &Style) -> Grid {
    let (max_abs, _) = survey(data.iter().flat_map(|z| [z.re, z.im]));
    let exp = data.len() == 1 && max_abs != 0. && !fits_fixed(max_abs, COMPLEX_FIXED_DIGITS, st);
    let scale = scale_of(max_abs, data.len(), COMPLEX_FIXED_DIGITS, st);
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
            if out.len() > MAX_OUTPUT_BYTES {
                return Err(OUTPUT_TOO_LARGE.into());
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

fn empty_body(rows: usize, cols: usize) -> String {
    if rows == 0 && cols == 0 {
        "     []\n".into()
    } else {
        format!("   Empty matrix: {rows}-by-{cols}\n")
    }
}

fn real_body(rows: usize, cols: usize, data: &[f64], fmt: &DisplayFormat) -> Result<String> {
    if data.is_empty() {
        return Ok(empty_body(rows, cols));
    }
    grid_body(&real_grid(rows, cols, data, &fmt.style())?, fmt)
}

fn complex_body(m: &ComplexMatrix, fmt: &DisplayFormat) -> Result<String> {
    if m.is_empty() {
        return Ok(empty_body(m.rows(), m.cols()));
    }
    check_size(m.len(), 20)?;
    grid_body(
        &complex_grid(m.rows(), m.cols(), m.data(), &fmt.style()),
        fmt,
    )
}

fn logical_body(m: &LogicalMatrix, fmt: &DisplayFormat) -> Result<String> {
    if m.is_empty() {
        return Ok(empty_body(m.rows(), m.cols()));
    }
    check_size(m.len(), 6)?;
    let data: Vec<f64> = m.data().iter().map(|&v| f64::from(v)).collect();
    real_body(m.rows(), m.cols(), &data, fmt)
}

/// Integer classes print every element right-aligned in the class's width
/// (`int8`: 5, `int32`: 12, `int64`: 21); single prints like double, with
/// 7 decimals under `format long`.
fn typed_body(a: &TypedArray, fmt: &DisplayFormat) -> Result<String> {
    if a.is_empty() {
        return Ok(empty_body(a.rows(), a.cols()));
    }
    if !a.is_integer() {
        // A lone single in `e` form is two characters narrower (`  3.4028e+38`).
        let st = if fmt.long {
            SINGLE_LONG
        } else {
            Style {
                exp_width: 12,
                ..SHORT
            }
        };
        let data = a.to_f64_vec()?;
        return grid_body(&real_grid(a.rows(), a.cols(), &data, &st)?, fmt);
    }
    // Class widths cover every representable value, including signed minima.
    // Reject oversized output before allocating any per-element strings.
    let width = a.class().display_width();
    check_size(a.len(), width)?;
    let grid = Grid {
        rows: a.rows(),
        cols: a.cols(),
        width,
        cells: (0..a.len())
            .map(|i| pad_left(&a.int(i).to_string(), width))
            .collect(),
        scale: None,
    };
    grid_body(&grid, fmt)
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
        let mut body = if c.rows() == 0 && c.cols() == 0 {
            "     ''\n".into()
        } else {
            empty_body(c.rows(), c.cols())
        };
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

fn cell_body(c: &CellArray, fmt: &DisplayFormat) -> Result<String> {
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
    for i in 0..c.len() {
        let element = c.get_linear(i)?;
        let summary = match cell_summary(element, &st) {
            Summary::Text(t) if c.len() == 1 && braced_empty(element) => {
                Summary::Text(format!("{{{t}}}"))
            }
            summary => summary,
        };
        bytes += summary.width() + 4;
        if bytes > MAX_OUTPUT_BYTES {
            return Err(OUTPUT_TOO_LARGE.into());
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
    Ok(wrap(name, false, &body, fmt))
}

pub fn echo_logical(name: &str, m: &LogicalMatrix, fmt: &DisplayFormat) -> Result<String> {
    Ok(wrap(name, false, &logical_body(m, fmt)?, fmt))
}

pub fn echo_complex(name: &str, m: &ComplexMatrix, fmt: &DisplayFormat) -> Result<String> {
    Ok(wrap(name, false, &complex_body(m, fmt)?, fmt))
}

/// Text of an unsuppressed `name = value` statement.
pub fn echo(name: &str, array: &Array, fmt: &DisplayFormat) -> Result<String> {
    match array {
        Array::Numeric(m) => echo_real(name, m, fmt),
        Array::Logical(m) => echo_logical(name, m, fmt),
        Array::Complex(m) => echo_complex(name, m, fmt),
        Array::Character(c) => Ok(wrap(name, false, &char_body(c, fmt), fmt)),
        Array::Cell(c) => Ok(wrap(name, true, &cell_body(c, fmt)?, fmt)),
        Array::Struct(s) => Ok(crate::struct_display::named(name, s, fmt)),
        Array::Graphics(g) => Ok(wrap(name, false, &format!("  {g}\n"), fmt)),
        Array::Handle(handle) => Ok(format!("{name} = \n\n    {}\n", handle.display())),
        Array::Typed(a) => Ok(wrap(name, false, &typed_body(a, fmt)?, fmt)),
    }
}

/// Text of `disp(value)`; `None` when MATLAB prints nothing (any empty value,
/// a 0x0 struct array too: Q7).
pub fn disp(array: &Array, fmt: &DisplayFormat) -> Result<Option<String>> {
    let (body, blank_after) = match array {
        a if a.is_empty() => return Ok(None),
        Array::Struct(s) => return Ok(Some(crate::struct_display::disp(s, fmt))),
        Array::Handle(handle) => return Ok(Some(format!("    {}", handle.display()))),
        Array::Numeric(m) => (real_body(m.rows(), m.cols(), m.data(), fmt)?, true),
        Array::Logical(m) => (logical_body(m, fmt)?, true),
        Array::Complex(m) => (complex_body(m, fmt)?, true),
        Array::Typed(a) => (typed_body(a, fmt)?, true),
        Array::Character(c) => (char_body(c, fmt), false),
        Array::Cell(c) => (cell_body(c, fmt)?, true),
        Array::Graphics(g) => (format!("{g}\n"), false),
    };
    let mut full = body;
    if blank_after && !fmt.compact {
        full.push('\n');
    }
    full.truncate(full.len() - 1);
    Ok(Some(full))
}
