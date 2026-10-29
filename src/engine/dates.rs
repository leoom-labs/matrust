//! Dates and times: datenum, datestr, datevec, weekday, etime, addtodate,
//! eomday and calendar. Serial date numbers follow MATLAB: day 1 is
//! 1-Jan-0000 in the proleptic Gregorian calendar, and a month or day out of
//! range rolls over (datenum(2016,13,1) is 1-Jan-2017). The clock functions
//! (now, clock, date) are not here: they depend on the time of day.
use super::{Engine, Value};
use crate::{Array, CharacterRow, Matrix, Result};

const NAMES: &[&str] = &[
    "datenum",
    "datestr",
    "datevec",
    "weekday",
    "etime",
    "addtodate",
    "eomday",
    "calendar",
];

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
/// Days before each month in a common year.
const BEFORE_MONTH: [f64; 12] = [
    0.0, 31.0, 59.0, 90.0, 120.0, 151.0, 181.0, 212.0, 243.0, 273.0, 304.0, 334.0,
];
/// datenum(1970, 1, 1): the civil-date conversion counts days from 1970.
const UNIX_DAY: i64 = 719_529;
/// Serial numbers beyond this (about 2.7e12 years) are rejected where a
/// calendar date must be built; the integer conversion stays exact.
const MAX_SERIAL: f64 = 1e15;

/// datestr's numeric format codes 0..31.
const FORMAT_CODES: [&str; 32] = [
    "dd-mmm-yyyy HH:MM:SS",
    "dd-mmm-yyyy",
    "mm/dd/yy",
    "mmm",
    "m",
    "mm",
    "mm/dd",
    "dd",
    "ddd",
    "d",
    "yyyy",
    "yy",
    "mmmyy",
    "HH:MM:SS",
    "HH:MM:SS PM",
    "HH:MM",
    "HH:MM PM",
    "QQ-yy",
    "QQ",
    "dd/mm",
    "dd/mm/yy",
    "mmm.dd,yyyy HH:MM:SS",
    "mmm.dd,yyyy",
    "mm/dd/yyyy",
    "dd/mm/yyyy",
    "yy/mm/dd",
    "yyyy/mm/dd",
    "QQ-yyyy",
    "mmmyyyy",
    "yyyy-mm-dd",
    "yyyymmddTHHMMSS",
    "yyyy-mm-dd HH:MM:SS",
];

/// Text forms tried, in order, when no format is given. Forms without a year
/// (MATLAB fills in the current year) are left out.
const TEXT_FORMS: &[&str] = &[
    "dd-mmm-yyyy HH:MM:SS",
    "dd-mmm-yyyy HH:MM",
    "dd-mmm-yyyy",
    "mm/dd/yy",
    "mm/dd/yyyy",
    "mm/dd/yy HH:MM:SS",
    "mm/dd/yyyy HH:MM:SS",
    "mmm.dd,yyyy HH:MM:SS",
    "mmm.dd,yyyy",
    "yyyy-mm-dd HH:MM:SS",
    "yyyy-mm-dd HH:MM",
    "yyyy-mm-dd",
    "yyyy/mm/dd HH:MM:SS",
    "yyyy/mm/dd",
];

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

/// Names with more than one output (`[n, s] = weekday(d)`, `[y, m, d] = datevec(t)`).
pub(super) fn multiple_outputs(name: &str) -> bool {
    matches!(name, "weekday" | "datevec")
}

fn arity(args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        Err("Not enough input arguments.".into())
    } else if args.len() > max {
        Err("Too many input arguments.".into())
    } else {
        Ok(())
    }
}

fn is_leap(y: f64) -> bool {
    y.rem_euclid(4.0) == 0.0 && (y.rem_euclid(100.0) != 0.0 || y.rem_euclid(400.0) == 0.0)
}

fn month_length(y: f64, month: usize) -> f64 {
    match month {
        2 if is_leap(y) => 29.0,
        2 => 28.0,
        4 | 6 | 9 | 11 => 30.0,
        _ => 31.0,
    }
}

/// Serial day number of year `y`, month `mo` (rolled into the year when out of
/// 1..12; a fraction of a month counts that month's days), day `d`.
fn serial_day(y: f64, mo: f64, d: f64) -> Result<f64> {
    if !(y.is_finite() && mo.is_finite()) {
        return Ok(y + mo + d);
    }
    if mo.abs() >= MAX_SERIAL {
        return Err("Date month must be below 1e15 in magnitude.".into());
    }
    let whole = mo.floor();
    let shift = ((whole - 1.0) / 12.0).floor();
    let y = y + shift;
    let month = (whole - 1.0).rem_euclid(12.0) as usize;
    let mut n = 365.0 * y + (y / 4.0).ceil() - (y / 100.0).ceil()
        + (y / 400.0).ceil()
        + BEFORE_MONTH[month]
        + d;
    if month > 1 && is_leap(y) {
        n += 1.0;
    }
    if mo != whole {
        n += (mo - whole) * month_length(y, month + 1);
    }
    Ok(n)
}

fn serial(parts: [f64; 6]) -> Result<f64> {
    let [y, mo, d, h, mi, s] = parts;
    let day = serial_day(y, mo, d)?;
    if h == 0.0 && mi == 0.0 && s == 0.0 {
        Ok(day)
    } else {
        Ok(day + (h * 3600.0 + mi * 60.0 + s) / 86400.0)
    }
}

/// (year, month 1..12, day 1..31) of an integer serial day.
fn civil(day: i64) -> (i64, usize, u32) {
    let z = day - UNIX_DAY + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as usize;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// One calendar instant, rounded to `per_second` units (1 or 1000).
struct Stamp {
    y: i64,
    month: usize,
    day: u32,
    weekday: usize,
    h: u64,
    mi: u64,
    s: u64,
    ms: u64,
}

fn stamp(t: f64, per_second: u64, who: &str) -> Result<Stamp> {
    if !t.is_finite() || t.abs() > MAX_SERIAL {
        return Err(format!(
            "{who}: date numbers must be finite and below 1e15 in magnitude"
        ));
    }
    let units_per_day = 86_400 * per_second;
    let mut day = t.floor();
    let mut units = ((t - day) * units_per_day as f64).round() as u64;
    if units >= units_per_day {
        units -= units_per_day;
        day += 1.0;
    }
    let day = day as i64;
    let (y, month, d) = civil(day);
    let seconds = units / per_second;
    Ok(Stamp {
        y,
        month,
        day: d,
        weekday: (day - 2).rem_euclid(7) as usize,
        h: seconds / 3600,
        mi: seconds / 60 % 60,
        s: seconds % 60,
        ms: if per_second == 1000 { units % 1000 } else { 0 },
    })
}

/// Date vector of a serial number, seconds rounded to the millisecond.
fn date_vector(t: f64) -> [f64; 6] {
    if !t.is_finite() || t.abs() > MAX_SERIAL {
        return [f64::NAN; 6];
    }
    let mut day = t.floor();
    let mut ms = ((t - day) * 86_400_000.0).round();
    if ms >= 86_400_000.0 {
        ms -= 86_400_000.0;
        day += 1.0;
    }
    let (y, month, d) = civil(day as i64);
    let h = (ms / 3_600_000.0).floor();
    let mi = ((ms - h * 3_600_000.0) / 60_000.0).floor();
    let s = (ms - h * 3_600_000.0 - mi * 60_000.0) / 1000.0;
    [y as f64, month as f64, f64::from(d), h, mi, s]
}

#[derive(Clone, Copy, PartialEq)]
enum Tok {
    Year4,
    Year2,
    MonthFull,
    MonthShort,
    Month2,
    MonthLetter,
    DayFull,
    DayShort,
    Day2,
    DayLetter,
    Hour,
    Minute,
    Second,
    Millis,
    AmPm,
    Quarter,
    Lit(char),
}

fn tokens(format: &str) -> Vec<Tok> {
    const TABLE: &[(&str, Tok)] = &[
        ("yyyy", Tok::Year4),
        ("yy", Tok::Year2),
        ("mmmm", Tok::MonthFull),
        ("mmm", Tok::MonthShort),
        ("mm", Tok::Month2),
        ("m", Tok::MonthLetter),
        ("dddd", Tok::DayFull),
        ("ddd", Tok::DayShort),
        ("dd", Tok::Day2),
        ("d", Tok::DayLetter),
        ("HH", Tok::Hour),
        ("MM", Tok::Minute),
        ("SS", Tok::Second),
        ("FFF", Tok::Millis),
        ("AM", Tok::AmPm),
        ("PM", Tok::AmPm),
        ("QQ", Tok::Quarter),
    ];
    let mut out = Vec::new();
    let mut rest = format;
    while let Some(c) = rest.chars().next() {
        if let Some((pattern, tok)) = TABLE.iter().find(|(p, _)| rest.starts_with(p)) {
            out.push(*tok);
            rest = &rest[pattern.len()..];
        } else {
            out.push(Tok::Lit(c));
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

fn render(tokens: &[Tok], s: &Stamp) -> String {
    let twelve = tokens.contains(&Tok::AmPm);
    let mut text = String::new();
    for tok in tokens {
        match *tok {
            Tok::Year4 => text.push_str(&format!("{:04}", s.y)),
            Tok::Year2 => text.push_str(&format!("{:02}", s.y.rem_euclid(100))),
            Tok::MonthFull => text.push_str(MONTHS[s.month - 1]),
            Tok::MonthShort => text.push_str(&MONTHS[s.month - 1][..3]),
            Tok::Month2 => text.push_str(&format!("{:02}", s.month)),
            Tok::MonthLetter => text.push_str(&MONTHS[s.month - 1][..1]),
            Tok::DayFull => text.push_str(DAYS[s.weekday]),
            Tok::DayShort => text.push_str(&DAYS[s.weekday][..3]),
            Tok::Day2 => text.push_str(&format!("{:02}", s.day)),
            Tok::DayLetter => text.push_str(&DAYS[s.weekday][..1]),
            Tok::Hour if twelve => {
                let h = if s.h.is_multiple_of(12) { 12 } else { s.h % 12 };
                text.push_str(&format!("{h:2}"));
            }
            Tok::Hour => text.push_str(&format!("{:02}", s.h)),
            Tok::Minute => text.push_str(&format!("{:02}", s.mi)),
            Tok::Second => text.push_str(&format!("{:02}", s.s)),
            Tok::Millis => text.push_str(&format!("{:03}", s.ms)),
            Tok::AmPm => text.push_str(if s.h < 12 { "AM" } else { "PM" }),
            Tok::Quarter => text.push_str(&format!("Q{}", (s.month - 1) / 3 + 1)),
            Tok::Lit(c) => text.push(c),
        }
    }
    text
}

/// Exact UTF-16 width, including variable month/day names and signed years.
/// Compute this without rendering or retaining any output rows.
fn render_width(tokens: &[Tok], s: &Stamp) -> usize {
    tokens
        .iter()
        .map(|tok| match tok {
            Tok::Year4 => {
                let digits = s.y.unsigned_abs().checked_ilog10().unwrap_or(0) as usize + 1;
                (digits + usize::from(s.y < 0)).max(4)
            }
            Tok::MonthFull => MONTHS[s.month - 1].len(),
            Tok::DayFull => DAYS[s.weekday].len(),
            Tok::MonthShort | Tok::DayShort | Tok::Millis => 3,
            Tok::MonthLetter | Tok::DayLetter => 1,
            Tok::Lit(c) => c.len_utf16(),
            _ => 2,
        })
        .sum()
}

/// Reads up to `max` ASCII digits; None when there is none.
fn digits(text: &[char], at: &mut usize, max: usize) -> Option<(f64, usize)> {
    let start = *at;
    while *at < text.len() && *at - start < max && text[*at].is_ascii_digit() {
        *at += 1;
    }
    if *at == start {
        return None;
    }
    let value: String = text[start..*at].iter().collect();
    Some((value.parse().ok()?, *at - start))
}

fn letters(text: &[char], at: &mut usize) -> String {
    let start = *at;
    while *at < text.len() && text[*at].is_alphabetic() {
        *at += 1;
    }
    text[start..*at].iter().collect::<String>().to_lowercase()
}

/// Parses one date text with a format; None when the text does not match.
/// `Err` for formats that cannot be read back (month or weekday letters,
/// quarters) or text without a year.
fn parse(text: &str, format: &str, pivot: f64, who: &str) -> Result<Option<[f64; 6]>> {
    let text: Vec<char> = text.trim().chars().collect();
    let tokens = tokens(format);
    let mut parts = [f64::NAN, 1.0, 1.0, 0.0, 0.0, 0.0];
    let mut afternoon = None;
    let mut at = 0;
    for tok in &tokens {
        let number = |at: &mut usize, max| digits(&text, at, max).map(|(v, _)| v);
        match *tok {
            Tok::Year4 => match number(&mut at, 4) {
                Some(v) => parts[0] = v,
                None => return Ok(None),
            },
            Tok::Year2 => match number(&mut at, 2) {
                Some(v) => {
                    let mut y = (pivot / 100.0).floor() * 100.0 + v;
                    if y < pivot {
                        y += 100.0;
                    }
                    parts[0] = y;
                }
                None => return Ok(None),
            },
            Tok::MonthFull | Tok::MonthShort => {
                let word = letters(&text, &mut at);
                let found = MONTHS.iter().position(|m| {
                    let m = m.to_lowercase();
                    if *tok == Tok::MonthShort {
                        word == m[..3]
                    } else {
                        word == m || word == m[..3]
                    }
                });
                match found {
                    Some(i) => parts[1] = (i + 1) as f64,
                    None => return Ok(None),
                }
            }
            Tok::DayFull | Tok::DayShort => {
                let word = letters(&text, &mut at);
                if !DAYS
                    .iter()
                    .any(|d| word == d.to_lowercase() || word == d[..3].to_lowercase())
                {
                    return Ok(None);
                }
            }
            Tok::Month2 | Tok::Day2 | Tok::Hour | Tok::Minute | Tok::Second => {
                let slot = match *tok {
                    Tok::Month2 => 1,
                    Tok::Day2 => 2,
                    Tok::Hour => 3,
                    Tok::Minute => 4,
                    _ => 5,
                };
                match number(&mut at, 2) {
                    Some(v) => parts[slot] = v,
                    None => return Ok(None),
                }
            }
            Tok::Millis => match digits(&text, &mut at, 3) {
                Some((v, len)) => parts[5] += v / 10f64.powi(len as i32),
                None => return Ok(None),
            },
            Tok::AmPm => {
                let word = letters(&text, &mut at);
                afternoon = match word.as_str() {
                    "am" => Some(false),
                    "pm" => Some(true),
                    _ => return Ok(None),
                };
            }
            Tok::MonthLetter | Tok::DayLetter | Tok::Quarter => {
                return Err(format!(
                    "{who}: the format '{format}' cannot be read back from text (m, d or QQ)"
                ));
            }
            Tok::Lit(c) if c.is_whitespace() => {
                while at < text.len() && text[at].is_whitespace() {
                    at += 1;
                }
            }
            Tok::Lit(c) => {
                if at < text.len() && text[at].eq_ignore_ascii_case(&c) {
                    at += 1;
                } else {
                    return Ok(None);
                }
            }
        }
    }
    if at != text.len() {
        return Ok(None);
    }
    if parts[0].is_nan() {
        return Err(format!(
            "{who}: date text without a year is not supported yet (MATLAB uses the current year)"
        ));
    }
    match afternoon {
        Some(true) if parts[3] < 12.0 => parts[3] += 12.0,
        Some(false) if parts[3] == 12.0 => parts[3] = 0.0,
        _ => {}
    }
    Ok(Some(parts))
}

/// The current year less 50: MATLAB's default window for two-digit years.
fn default_pivot() -> f64 {
    let days = crate::clock::system_now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400);
    civil(UNIX_DAY + days as i64).0 as f64 - 50.0
}

fn looks_like_date_vectors(m: &Matrix) -> bool {
    m.cols() == 6
        && m.rows() > 0
        && (0..m.rows()).all(|r| {
            let (y, mo, d) = (m.get(r, 0), m.get(r, 1), m.get(r, 2));
            y.fract() == 0.0
                && (y - 2000.0).abs() < 10_000.0
                && (1.0..=12.0).contains(&mo)
                && mo.fract() == 0.0
                && (1.0..=31.0).contains(&d)
        })
}

impl Engine {
    /// Rows of a date-vector matrix (3 or 6 columns) as serial numbers.
    fn datex_rows_to_serial(&self, m: &Matrix) -> Result<Vec<f64>> {
        (0..m.rows())
            .map(|r| {
                if r % 65_536 == 65_535 {
                    self.check_cancel()?;
                }
                let mut parts = [0.0; 6];
                for (c, part) in parts.iter_mut().enumerate().take(m.cols()) {
                    *part = m.get(r, c);
                }
                serial(parts)
            })
            .collect()
    }

    /// Single-output entry for `fn call`; None when `name` is not in this family.
    pub(super) fn datex_call(&mut self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if !handles(name) {
            return Ok(None);
        }
        Ok(Some(self.datex_outputs(name, args, 1)?.remove(0)))
    }

    /// `count` outputs of a call of this family.
    pub(super) fn datex_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let max_outputs = match name {
            "weekday" => 2,
            "datevec" => 6,
            _ => 1,
        };
        if count > max_outputs {
            return Err("Too many output arguments.".into());
        }
        match name {
            "datenum" => Ok(vec![self.datex_datenum(args)?]),
            "datestr" => Ok(vec![self.datex_datestr(args)?]),
            "datevec" => self.datex_datevec(args, count),
            "weekday" => self.datex_weekday(args, count),
            "etime" => Ok(vec![self.datex_etime(args)?]),
            "addtodate" => Ok(vec![self.datex_addtodate(args)?]),
            "eomday" => Ok(vec![self.datex_eomday(args)?]),
            _ => Ok(vec![self.datex_calendar(args)?]),
        }
    }

    fn datex_real(&self, value: &Value, who: &str) -> Result<Matrix> {
        match value {
            Value::Matrix(m) => Ok(m.clone()),
            Value::Logical(m) => m.to_numeric(&self.cancel),
            Value::Complex(_) => Err(format!("{who}: input must be real")),
            _ => Err(format!("{who}: input must be numeric")),
        }
    }

    /// The rows of a character argument (trailing blanks kept off); None for
    /// any other value.
    fn datex_text(&self, value: &Value) -> Option<Vec<String>> {
        match value {
            Value::Text(s) | Value::QuotedString(s) => Some(vec![s.clone()]),
            Value::Container(Array::Character(c)) => {
                let (rows, cols) = (c.rows(), c.cols());
                Some(
                    (0..rows)
                        .map(|r| {
                            let row: Vec<u16> =
                                (0..cols).map(|k| c.units()[k * rows + r]).collect();
                            String::from_utf16_lossy(&row)
                                .trim_end_matches([' ', '\0'])
                                .to_string()
                        })
                        .collect(),
                )
            }
            _ => None,
        }
    }

    fn datex_string(&self, value: &Value, who: &str, what: &str) -> Result<String> {
        match self.datex_text(value) {
            Some(rows) if rows.len() == 1 => Ok(rows.into_iter().next().unwrap_or_default()),
            _ => Err(format!("{who}: {what} must be a character vector")),
        }
    }

    /// Serial numbers of date text, one per row, read with `format` or the
    /// default text forms.
    fn datex_parse_rows(
        &self,
        rows: &[String],
        format: Option<&str>,
        pivot: f64,
        who: &str,
    ) -> Result<Vec<[f64; 6]>> {
        let mut out = Vec::with_capacity(rows.len());
        for (i, row) in rows.iter().enumerate() {
            if i % 1024 == 1023 {
                self.check_cancel()?;
            }
            let parsed = match format {
                Some(format) => parse(row, format, pivot, who)?,
                None => {
                    let mut found = None;
                    for form in TEXT_FORMS {
                        if let Some(parts) = parse(row, form, pivot, who)? {
                            found = Some(parts);
                            break;
                        }
                    }
                    found
                }
            };
            match parsed {
                Some(parts) => out.push(parts),
                None => {
                    return Err(match format {
                        Some(format) => {
                            format!("{who}: the text '{row}' does not match the format '{format}'")
                        }
                        None => format!(
                            "{who}: cannot read '{row}' as a date; give its format, as in {who}(text, 'yyyy-mm-dd')"
                        ),
                    });
                }
            }
        }
        Ok(out)
    }

    /// Date vectors of a text argument with the optional format or pivot year
    /// that follows it; None when `args[0]` is not text.
    fn datex_text_vectors(&self, args: &[Value], who: &str) -> Result<Option<Vec<[f64; 6]>>> {
        let Some(rows) = self.datex_text(&args[0]) else {
            return Ok(None);
        };
        let mut format = None;
        let mut pivot = None;
        for arg in &args[1..] {
            if let Some(text) = self.datex_text(arg) {
                if format.is_some() {
                    return Err("Too many input arguments.".into());
                }
                format = Some(
                    text.into_iter()
                        .next()
                        .ok_or(format!("{who}: the format must be a character vector"))?,
                );
            } else {
                let m = self.datex_real(arg, who)?;
                if m.len() != 1 || pivot.is_some() {
                    return Err(format!("{who}: the pivot year must be a scalar"));
                }
                pivot = Some(m.data()[0]);
            }
        }
        let format = format.filter(|f| !f.is_empty());
        let pivot = pivot.unwrap_or_else(default_pivot);
        self.datex_parse_rows(&rows, format.as_deref(), pivot, who)
            .map(Some)
    }

    fn datex_column(&self, values: Vec<f64>) -> Result<Value> {
        Ok(Value::Matrix(Matrix::from_column_vec(
            values.len(),
            1,
            values,
        )?))
    }

    fn datex_datenum(&self, args: &[Value]) -> Result<Value> {
        let who = "datenum";
        arity(args, 1, 6)?;
        if let Some(vectors) = self.datex_text_vectors(args, who)? {
            if args.len() > 3 {
                return Err("Too many input arguments.".into());
            }
            return self.datex_column(vectors.into_iter().map(serial).collect::<Result<_>>()?);
        }
        let parts: Vec<Matrix> = args
            .iter()
            .map(|a| self.datex_real(a, who))
            .collect::<Result<_>>()?;
        match parts.len() {
            1 => {
                let m = &parts[0];
                if m.cols() == 3 || m.cols() == 6 {
                    self.datex_column(self.datex_rows_to_serial(m)?)
                } else {
                    Ok(Value::Matrix(m.clone()))
                }
            }
            2 => Err("datenum: give year, month and day, a date vector or date text".into()),
            _ => {
                let (rows, cols) = self.datex_common_shape(&parts, who)?;
                let len = rows * cols;
                let mut data = Vec::with_capacity(len);
                for i in 0..len {
                    if i % 65_536 == 65_535 {
                        self.check_cancel()?;
                    }
                    let mut v = [0.0; 6];
                    for (slot, m) in v.iter_mut().zip(&parts) {
                        *slot = if m.len() == 1 {
                            m.data()[0]
                        } else {
                            m.data()[i]
                        };
                    }
                    data.push(serial(v)?);
                }
                Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
            }
        }
    }

    /// Shape shared by every non-scalar argument.
    fn datex_common_shape(&self, parts: &[Matrix], who: &str) -> Result<(usize, usize)> {
        let mut shape = (1, 1);
        for m in parts {
            if m.len() != 1 {
                if shape != (1, 1) && shape != (m.rows(), m.cols()) {
                    return Err(format!("{who}: non-scalar inputs must have the same size"));
                }
                shape = (m.rows(), m.cols());
            }
        }
        if parts.iter().any(|m| m.is_empty()) {
            return Ok((0, 0));
        }
        Ok(shape)
    }

    /// Serial numbers of a date argument. Only datestr interprets plausible
    /// six-column matrices as date vectors; other numeric inputs are serials.
    fn datex_serials(&self, args: &[Value], who: &str) -> Result<(Vec<f64>, (usize, usize))> {
        if let Some(vectors) = self.datex_text_vectors(args, who)? {
            let n = vectors.len();
            return Ok((
                vectors.into_iter().map(serial).collect::<Result<_>>()?,
                (n, 1),
            ));
        }
        let m = self.datex_real(&args[0], who)?;
        if who == "datestr" && looks_like_date_vectors(&m) {
            return Ok((self.datex_rows_to_serial(&m)?, (m.rows(), 1)));
        }
        Ok((m.data().to_vec(), (m.rows(), m.cols())))
    }

    /// A character matrix of `rows`, blank-padded on the right; text for one row.
    fn datex_char_rows(&self, rows: Vec<String>) -> Result<Value> {
        if rows.len() == 1 {
            return Ok(Value::Text(rows.into_iter().next().unwrap_or_default()));
        }
        let units: Vec<Vec<u16>> = rows.iter().map(|r| r.encode_utf16().collect()).collect();
        let width = units.iter().map(Vec::len).max().unwrap_or(0);
        let count = units.len();
        CharacterRow::checked_len(count, width)?;
        let mut data = vec![u16::from(b' '); count * width];
        for (r, row) in units.iter().enumerate() {
            for (k, &u) in row.iter().enumerate() {
                data[k * count + r] = u;
            }
        }
        Ok(Value::Container(Array::Character(
            CharacterRow::from_column_major(count, width, data, &self.cancel)?,
        )))
    }

    fn datex_datestr(&self, args: &[Value]) -> Result<Value> {
        let who = "datestr";
        arity(args, 1, 3)?;
        // Date text is read with the default text forms; a second argument is
        // always the output format.
        let (serials, _) = self.datex_serials(&args[..1], who)?;
        let format = match args.get(1) {
            None => None,
            Some(value) => match self.datex_text(value) {
                Some(_) => Some(self.datex_string(value, who, "the format")?),
                None => {
                    let m = self.datex_real(value, who)?;
                    let code = m.data().first().copied().unwrap_or(f64::NAN);
                    if m.len() != 1 || code.fract() != 0.0 || !(0.0..=31.0).contains(&code) {
                        return Err(format!(
                            "{who}: a numeric format must be one of the codes 0 to 31"
                        ));
                    }
                    Some(FORMAT_CODES[code as usize].to_string())
                }
            },
        };
        if args.len() == 3 {
            return Err(format!(
                "{who}: the pivot year argument is not supported yet"
            ));
        }
        let format = match format {
            Some(f) if !f.is_empty() => f,
            _ if serials.iter().all(|t| t.fract() == 0.0) => FORMAT_CODES[1].into(),
            _ => FORMAT_CODES[0].into(),
        };
        let tokens = tokens(&format);
        let per_second = if tokens.contains(&Tok::Millis) {
            1000
        } else {
            1
        };
        let mut width = 0;
        for (i, &t) in serials.iter().enumerate() {
            if i % 4096 == 4095 {
                self.check_cancel()?;
            }
            width = width.max(render_width(&tokens, &stamp(t, per_second, who)?));
            CharacterRow::checked_len(serials.len(), width)?;
        }
        let mut rows = Vec::with_capacity(serials.len());
        for (i, &t) in serials.iter().enumerate() {
            if i % 4096 == 4095 {
                self.check_cancel()?;
            }
            rows.push(render(&tokens, &stamp(t, per_second, who)?));
        }
        if rows.is_empty() {
            return Ok(Value::Text(String::new()));
        }
        self.datex_char_rows(rows)
    }

    fn datex_datevec(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let who = "datevec";
        arity(args, 1, 3)?;
        let n = match &args[0] {
            Value::Text(_) | Value::QuotedString(_) => 1,
            Value::Container(Array::Character(c)) => c.rows(),
            Value::Matrix(m) => m.len(),
            Value::Logical(m) => m.len(),
            value => self.datex_real(value, who)?.len(),
        };
        Matrix::checked_len(n, 6)?;
        let vectors: Vec<[f64; 6]> = match self.datex_text_vectors(args, who)? {
            Some(vectors) => vectors,
            None => {
                if args.len() > 1 {
                    return Err("Too many input arguments.".into());
                }
                let (serials, _) = self.datex_serials(args, who)?;
                let mut vectors = Vec::with_capacity(serials.len());
                for (i, t) in serials.into_iter().enumerate() {
                    if i % 65_536 == 65_535 {
                        self.check_cancel()?;
                    }
                    vectors.push(date_vector(t));
                }
                vectors
            }
        };
        let n = vectors.len();
        if count <= 1 {
            let mut data = vec![0.0; n * 6];
            for (r, v) in vectors.iter().enumerate() {
                for (c, &x) in v.iter().enumerate() {
                    data[c * n + r] = x;
                }
            }
            return Ok(vec![Value::Matrix(Matrix::from_column_vec(n, 6, data)?)]);
        }
        (0..count)
            .map(|c| self.datex_column(vectors.iter().map(|v| v[c]).collect()))
            .collect()
    }

    fn datex_weekday(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let who = "weekday";
        arity(args, 1, 2)?;
        let long = match args.get(1) {
            None => false,
            Some(value) => match self.datex_string(value, who, "the form")?.as_str() {
                "long" => true,
                "short" => false,
                other => {
                    return Err(format!(
                        "{who}: the form must be 'long' or 'short', not '{other}'"
                    ));
                }
            },
        };
        let (serials, (rows, cols)) = self.datex_serials(&args[..1], who)?;
        let numbers: Vec<f64> = serials
            .iter()
            .enumerate()
            .map(|(i, &t)| {
                if i % 65_536 == 65_535 {
                    self.check_cancel()?;
                }
                Ok(if t.is_finite() {
                    (t.floor() - 2.0).rem_euclid(7.0) + 1.0
                } else {
                    f64::NAN
                })
            })
            .collect::<Result<_>>()?;
        let mut outputs = vec![Value::Matrix(Matrix::from_column_vec(
            rows,
            cols,
            numbers.clone(),
        )?)];
        if count > 1 {
            let names = self.datex_weekday_names(&numbers, long)?;
            outputs.push(if names.is_empty() {
                Value::Text(String::new())
            } else {
                self.datex_char_rows(names)?
            });
        }
        Ok(outputs)
    }

    fn datex_weekday_names(&self, numbers: &[f64], long: bool) -> Result<Vec<String>> {
        numbers
            .iter()
            .enumerate()
            .map(|(i, &n)| {
                if i % 4096 == 4095 {
                    self.check_cancel()?;
                }
                if n.is_nan() {
                    return Err("weekday: date numbers must be finite for day names".into());
                }
                let day = DAYS[n as usize - 1];
                Ok(if long { day } else { &day[..3] }.to_string())
            })
            .collect()
    }

    fn datex_etime(&self, args: &[Value]) -> Result<Value> {
        let who = "etime";
        arity(args, 2, 2)?;
        let (t1, t0) = (
            self.datex_real(&args[0], who)?,
            self.datex_real(&args[1], who)?,
        );
        if t1.cols() != 6 || t0.cols() != 6 || t1.rows() != t0.rows() {
            return Err(format!(
                "{who}: inputs must be date vectors with 6 columns and the same number of rows"
            ));
        }
        let seconds = (0..t1.rows())
            .map(|r| {
                if r % 65_536 == 65_535 {
                    self.check_cancel()?;
                }
                let days = serial_day(t1.get(r, 0), t1.get(r, 1), t1.get(r, 2))?
                    - serial_day(t0.get(r, 0), t0.get(r, 1), t0.get(r, 2))?;
                Ok(86_400.0 * days
                    + (t1.get(r, 3) - t0.get(r, 3)) * 3600.0
                    + (t1.get(r, 4) - t0.get(r, 4)) * 60.0
                    + (t1.get(r, 5) - t0.get(r, 5)))
            })
            .collect::<Result<_>>()?;
        self.datex_column(seconds)
    }

    fn datex_addtodate(&self, args: &[Value]) -> Result<Value> {
        let who = "addtodate";
        arity(args, 3, 3)?;
        let dates = self.datex_real(&args[0], who)?;
        let quantity = self.datex_real(&args[1], who)?;
        let field = self.datex_string(&args[2], who, "the field")?;
        if quantity.len() != 1 && (quantity.rows(), quantity.cols()) != (dates.rows(), dates.cols())
        {
            return Err(format!(
                "{who}: the quantity must be a scalar or the size of the dates"
            ));
        }
        let q = |i: usize| {
            if quantity.len() == 1 {
                quantity.data()[0]
            } else {
                quantity.data()[i]
            }
        };
        let scale = match field.to_lowercase().as_str() {
            "year" | "month" => None,
            "day" => Some(1.0),
            "hour" => Some(24.0),
            "minute" => Some(1440.0),
            "second" => Some(86_400.0),
            "millisecond" => Some(86_400_000.0),
            _ => {
                return Err(format!(
                    "{who}: the field must be 'year', 'month', 'day', 'hour', 'minute', 'second' or 'millisecond'"
                ));
            }
        };
        let months_per = if field.eq_ignore_ascii_case("year") {
            12.0
        } else {
            1.0
        };
        let mut data = Vec::with_capacity(dates.len());
        for (i, &t) in dates.data().iter().enumerate() {
            if i % 65_536 == 65_535 {
                self.check_cancel()?;
            }
            let amount = q(i);
            data.push(match scale {
                Some(per_day) => t + amount / per_day,
                None => {
                    if amount.fract() != 0.0 {
                        return Err(format!(
                            "{who}: a quantity of years or months must be an integer"
                        ));
                    }
                    if !t.is_finite() || t.abs() > MAX_SERIAL {
                        f64::NAN
                    } else {
                        let day = t.floor();
                        let (y, month, d) = civil(day as i64);
                        let total = month as f64 - 1.0 + amount * months_per;
                        let y = y as f64 + (total / 12.0).floor();
                        let month = total.rem_euclid(12.0) as usize + 1;
                        let d = f64::from(d).min(month_length(y, month));
                        serial_day(y, month as f64, d)? + (t - day)
                    }
                }
            });
        }
        Ok(Value::Matrix(Matrix::from_column_vec(
            dates.rows(),
            dates.cols(),
            data,
        )?))
    }

    fn datex_eomday(&self, args: &[Value]) -> Result<Value> {
        let who = "eomday";
        arity(args, 2, 2)?;
        let parts = [
            self.datex_real(&args[0], who)?,
            self.datex_real(&args[1], who)?,
        ];
        let (rows, cols) = self.datex_common_shape(&parts, who)?;
        let pick = |m: &Matrix, i: usize| {
            if m.len() == 1 {
                m.data()[0]
            } else {
                m.data()[i]
            }
        };
        let mut data = Vec::with_capacity(rows * cols);
        for i in 0..rows * cols {
            if i % 65_536 == 65_535 {
                self.check_cancel()?;
            }
            let (y, m) = (pick(&parts[0], i), pick(&parts[1], i));
            data.push(
                if y.is_finite() && m.fract() == 0.0 && (1.0..=12.0).contains(&m) {
                    month_length(y, m as usize)
                } else {
                    f64::NAN
                },
            );
        }
        Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
    }

    fn datex_calendar(&self, args: &[Value]) -> Result<Value> {
        let who = "calendar";
        let (y, month) = match args.len() {
            0 => {
                return Err(format!(
                    "{who}: give a year and month or a date; the current month is not supported yet"
                ));
            }
            1 => {
                let (serials, _) = self.datex_serials(args, who)?;
                let [t] = serials[..] else {
                    return Err(format!("{who}: the date must be a single date"));
                };
                let v = date_vector(t);
                if v[0].is_nan() {
                    return Err(format!("{who}: the date must be finite"));
                }
                (v[0], v[1] as usize)
            }
            2 => {
                let y = self.datex_real(&args[0], who)?;
                let m = self.datex_real(&args[1], who)?;
                if y.len() != 1 || m.len() != 1 {
                    return Err("YEAR and MONTH must be scalars.".into());
                }
                let (y, m) = (y.data()[0], m.data()[0]);
                if !y.is_finite() || y.fract() != 0.0 || y.abs() > 1e12 {
                    return Err(format!("{who}: the year must be a finite integer"));
                }
                if m.fract() != 0.0 || !(1.0..=12.0).contains(&m) {
                    return Err(format!("{who}: the month must be an integer from 1 to 12"));
                }
                (y, m as usize)
            }
            _ => return Err("Too many input arguments.".into()),
        };
        let first = serial_day(y, month as f64, 1.0)?;
        let offset = (first - 2.0).rem_euclid(7.0) as usize;
        let mut data = vec![0.0; 42];
        for day in 0..month_length(y, month) as usize {
            let cell = offset + day;
            let (r, c) = (cell / 7, cell % 7);
            data[c * 6 + r] = (day + 1) as f64;
        }
        Ok(Value::Matrix(Matrix::from_column_vec(6, 7, data)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_loops_observe_cancellation() {
        let e = Engine::default();
        let dates = Matrix::from_column_vec(65_536, 6, vec![1.0; 65_536 * 6]).unwrap();
        let serials = Matrix::from_column_vec(65_536, 1, vec![1.0; 65_536]).unwrap();
        e.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        // Call the loops directly so the dispatcher's entry check cannot mask
        // missing checkpoints. No timing-dependent cancellation thread needed.
        assert_eq!(
            e.datex_rows_to_serial(&dates).unwrap_err(),
            "Calculation cancelled"
        );
        assert_eq!(
            e.datex_etime(&[Value::Matrix(dates.clone()), Value::Matrix(dates)])
                .unwrap_err(),
            "Calculation cancelled"
        );
        assert_eq!(
            e.datex_weekday(&[Value::Matrix(serials)], 1).unwrap_err(),
            "Calculation cancelled"
        );
        assert_eq!(
            e.datex_weekday_names(&[1.0; 4096], true).unwrap_err(),
            "Calculation cancelled"
        );
    }

    #[test]
    fn datevec_checks_size_before_converting_elements() {
        let e = Engine::default();
        let n = crate::matrix::MAX_ELEMENTS / 6 + 1;
        let input = Value::Matrix(Matrix::from_column_vec(n, 1, vec![0.0; n]).unwrap());
        e.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        // A conversion checkpoint would return cancellation. The shape error
        // must win before allocating/converting six components per element.
        assert_eq!(
            e.datex_datevec(&[input], 1).unwrap_err(),
            Matrix::checked_len(n, 6).unwrap_err()
        );
    }

    #[test]
    fn preflight_width_matches_utf16_rendering() {
        let formats = FORMAT_CODES
            .iter()
            .copied()
            .chain(["yyyy mmmm dddd 😀", "m d QQ FFF PM"]);
        for format in formats {
            let tokens = tokens(format);
            for t in [
                -MAX_SERIAL,
                -800_000.5,
                1.0,
                736_390.75,
                736_390.9999999,
                MAX_SERIAL,
            ] {
                let s = stamp(t, 1000, "test").unwrap();
                assert_eq!(
                    render_width(&tokens, &s),
                    render(&tokens, &s).encode_utf16().count(),
                    "{format}, {t}"
                );
            }
        }
    }

    #[test]
    fn serial_days_match_matlab_anchors() {
        assert_eq!(serial_day(0.0, 1.0, 1.0).unwrap(), 1.0);
        assert_eq!(serial_day(1970.0, 1.0, 1.0).unwrap(), UNIX_DAY as f64);
        assert_eq!(serial_day(2016.0, 3.0, 1.0).unwrap(), 736_390.0);
        assert_eq!(serial_day(2016.0, 13.0, 1.0).unwrap(), 736_696.0);
        assert_eq!(serial_day(2016.0, 0.0, 1.0), serial_day(2015.0, 12.0, 1.0));
        for day in [-800_000_i64, -1, 0, 1, 60, 61, 736_390, 3_000_000] {
            let (y, m, d) = civil(day);
            assert_eq!(
                serial_day(y as f64, m as f64, f64::from(d)).unwrap(),
                day as f64
            );
        }
    }

    #[test]
    fn text_forms_round_trip() {
        let p = parse("01-Mar-2016 12:30:00", TEXT_FORMS[0], 1976.0, "t")
            .unwrap()
            .unwrap();
        assert_eq!(p, [2016.0, 3.0, 1.0, 12.0, 30.0, 0.0]);
        assert_eq!(parse("03/01/2016", "mm/dd/yy", 1976.0, "t").unwrap(), None);
        let p = parse("03/01/16", "mm/dd/yy", 1976.0, "t").unwrap().unwrap();
        assert_eq!(p[0], 2016.0);
        let p = parse("6:15 PM", "HH:MM PM", 1976.0, "t");
        assert!(p.is_err(), "no year");
    }
}
