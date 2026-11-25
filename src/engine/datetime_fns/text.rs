//! datetime display formats: letters y u M d D e E H h m s S a, quoted
//! literals, English month and day names (no locale). This is not the datestr
//! parser: here MM is the month and mm the minute.
use super::super::dates::{DAYS, MONTHS};
use super::calendar::{self, Parts, month_length};
use crate::Result;
use crate::datetime_array::DateTimeValue;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Tok {
    Year(usize),
    Month(usize),
    Day(usize),
    DayOfYear(usize),
    Weekday(usize),
    Hour24(usize),
    Hour12(usize),
    Minute(usize),
    Second(usize),
    Frac(usize),
    AmPm,
    Lit(String),
}

impl Tok {
    fn numeric(&self) -> bool {
        match self {
            Self::Month(n) | Self::Weekday(n) => *n <= 2,
            Self::Lit(_) | Self::AmPm => false,
            _ => true,
        }
    }
}

/// Tokens of a display format; an unknown letter is an error.
pub(crate) fn tokens(format: &str) -> Result<Vec<Tok>> {
    let chars: Vec<char> = format.chars().collect();
    let mut out: Vec<Tok> = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // Quoted literal; '' inside or outside quotes is one quote.
            if chars.get(i + 1) == Some(&'\'') {
                lit.push('\'');
                i += 2;
                continue;
            }
            i += 1;
            loop {
                match chars.get(i) {
                    None => {
                        return Err(format!(
                            "Unterminated quoted text in datetime format '{format}'"
                        ));
                    }
                    Some('\'') if chars.get(i + 1) == Some(&'\'') => {
                        lit.push('\'');
                        i += 2;
                    }
                    Some('\'') => {
                        i += 1;
                        break;
                    }
                    Some(&ch) => {
                        lit.push(ch);
                        i += 1;
                    }
                }
            }
            continue;
        }
        if !c.is_ascii_alphabetic() {
            lit.push(c);
            i += 1;
            continue;
        }
        let mut n = 1;
        while chars.get(i + n) == Some(&c) {
            n += 1;
        }
        let tok = match c {
            'y' | 'u' => Tok::Year(n),
            'M' | 'L' if n <= 5 => Tok::Month(n),
            'd' if n <= 2 => Tok::Day(n),
            'D' if n <= 3 => Tok::DayOfYear(n),
            'e' | 'E' if n <= 5 => Tok::Weekday(if c == 'E' && n <= 3 { 3 } else { n }),
            'H' if n <= 2 => Tok::Hour24(n),
            'h' if n <= 2 => Tok::Hour12(n),
            'm' if n <= 2 => Tok::Minute(n),
            's' if n <= 2 => Tok::Second(n),
            'S' if n <= 9 => Tok::Frac(n),
            'a' if n == 1 => Tok::AmPm,
            _ => {
                return Err(format!(
                    "Unsupported datetime format '{}' in '{format}'",
                    chars[i..i + n].iter().collect::<String>()
                ));
            }
        };
        if !lit.is_empty() {
            out.push(Tok::Lit(std::mem::take(&mut lit)));
        }
        out.push(tok);
        i += n;
    }
    if !lit.is_empty() {
        out.push(Tok::Lit(lit));
    }
    Ok(out)
}

fn pad(out: &mut String, v: i64, width: usize) {
    if v < 0 {
        out.push('-');
        out.push_str(&format!("{:0width$}", -v));
    } else {
        out.push_str(&format!("{v:0width$}"));
    }
}

/// Fractional digits to show: the widest `S` run, or 0 when seconds are shown
/// with no fraction. `None` when the format does not show seconds.
fn displayed_fraction_digits(tokens: &[Tok]) -> Option<u32> {
    let mut frac = None;
    let mut seconds = false;
    for tok in tokens {
        match tok {
            Tok::Frac(n) => frac = Some(frac.map_or(*n as u32, |f: u32| f.max(*n as u32))),
            Tok::Second(_) => seconds = true,
            _ => {}
        }
    }
    frac.or(seconds.then_some(0))
}

/// Round the nanosecond field to `digits` (half away from zero) and carry
/// into seconds, minutes, hours and days. Display only; the stored value
/// is unchanged.
fn round_fraction(v: DateTimeValue, digits: u32) -> DateTimeValue {
    let unit = 10i128.pow(9 - digits);
    let ns = i128::from(v.nanos);
    let rem = ns.rem_euclid(unit);
    let rounded = ns - rem + if rem * 2 >= unit { unit } else { 0 };
    let delta = rounded - ns;
    if delta == 0 {
        return v;
    }
    v.total_nanos()
        .and_then(|total| calendar::from_total(total + delta).ok())
        .unwrap_or(v)
}

/// Text of one value (`NaT` for NaT).
pub(crate) fn render(tokens: &[Tok], v: DateTimeValue) -> String {
    if v.is_nat() {
        return "NaT".into();
    }
    let v = match displayed_fraction_digits(tokens) {
        Some(digits) => round_fraction(v, digits),
        None => v,
    };
    let p: Parts = calendar::parts(v);
    let mut out = String::new();
    for tok in tokens {
        match tok {
            Tok::Year(2) => pad(&mut out, p.y.rem_euclid(100), 2),
            Tok::Year(n) => pad(&mut out, p.y, *n),
            Tok::Month(n) => match n {
                1 | 2 => pad(&mut out, p.mo as i64, *n),
                3 => out.push_str(&MONTHS[p.mo - 1][..3]),
                4 => out.push_str(MONTHS[p.mo - 1]),
                _ => out.push_str(&MONTHS[p.mo - 1][..1]),
            },
            Tok::Day(n) => pad(&mut out, i64::from(p.d), *n),
            Tok::DayOfYear(n) => pad(&mut out, i64::from(p.doy), *n),
            Tok::Weekday(n) => match n {
                1 | 2 => pad(&mut out, p.weekday as i64, *n),
                3 => out.push_str(&DAYS[p.weekday - 1][..3]),
                4 => out.push_str(DAYS[p.weekday - 1]),
                _ => out.push_str(&DAYS[p.weekday - 1][..1]),
            },
            Tok::Hour24(n) => pad(&mut out, i64::from(p.h), *n),
            Tok::Hour12(n) => {
                let h = if p.h.is_multiple_of(12) { 12 } else { p.h % 12 };
                pad(&mut out, i64::from(h), *n);
            }
            Tok::Minute(n) => pad(&mut out, i64::from(p.mi), *n),
            Tok::Second(n) => pad(&mut out, i64::from(p.s), *n),
            Tok::Frac(n) => {
                let digits = format!("{:09}", p.ns);
                out.push_str(&digits[..*n]);
            }
            Tok::AmPm => out.push_str(if p.h < 12 { "AM" } else { "PM" }),
            Tok::Lit(s) => out.push_str(s),
        }
    }
    out
}

/// Fields read from text; missing ones take defaults at the end.
#[derive(Default)]
struct Fields {
    year: Option<i64>,
    month: Option<i64>,
    day: Option<i64>,
    doy: Option<i64>,
    hour: Option<i64>,
    hour12: bool,
    pm: Option<bool>,
    minute: i64,
    second: i64,
    frac: i64,
}

fn read_digits(text: &[char], at: &mut usize, max: usize) -> Option<(i64, usize)> {
    let start = *at;
    let mut v: i64 = 0;
    while *at < text.len() && *at - start < max && text[*at].is_ascii_digit() {
        v = v * 10 + i64::from(text[*at].to_digit(10)?);
        *at += 1;
    }
    (*at > start).then_some((v, *at - start))
}

fn read_name(text: &[char], at: &mut usize, names: &[&str]) -> Option<usize> {
    let rest: String = text[*at..].iter().collect::<String>().to_lowercase();
    // Full names first, then three-letter abbreviations.
    for (i, name) in names.iter().enumerate() {
        let lower = name.to_lowercase();
        if rest.starts_with(&lower) {
            *at += name.chars().count();
            return Some(i);
        }
    }
    for (i, name) in names.iter().enumerate() {
        let lower: String = name.to_lowercase().chars().take(3).collect();
        if rest.starts_with(&lower) {
            *at += 3;
            return Some(i);
        }
    }
    None
}

/// Value of `text` read with `tokens`; None when it does not match.
/// `pivot` is the first year of the 100-year window for two-digit years and
/// `current_year` fills a format without a year.
pub(crate) fn parse(
    tokens: &[Tok],
    text: &str,
    pivot: i64,
    current_year: i64,
) -> Option<DateTimeValue> {
    let text: Vec<char> = text.trim().chars().collect();
    let mut at = 0;
    let mut f = Fields::default();
    for (i, tok) in tokens.iter().enumerate() {
        let adjacent = tokens.get(i + 1).is_some_and(Tok::numeric);
        match tok {
            Tok::Lit(s) => {
                for want in s.chars() {
                    let got = *text.get(at)?;
                    if !got.eq_ignore_ascii_case(&want) {
                        return None;
                    }
                    at += 1;
                }
            }
            Tok::Year(2) => {
                let (yy, _) = read_digits(&text, &mut at, 2)?;
                let mut y = pivot - pivot.rem_euclid(100) + yy;
                if y < pivot {
                    y += 100;
                }
                f.year = Some(y);
            }
            Tok::Year(n) => {
                let negative = text.get(at) == Some(&'-') && *n != 2;
                if negative {
                    at += 1;
                }
                let max = if adjacent { (*n).max(4) } else { 9 };
                let (y, _) = read_digits(&text, &mut at, max)?;
                f.year = Some(if negative { -y } else { y });
            }
            Tok::Month(n) if *n <= 2 => f.month = Some(read_digits(&text, &mut at, 2)?.0),
            Tok::Month(_) => f.month = Some(read_name(&text, &mut at, &MONTHS)? as i64 + 1),
            Tok::Day(_) => f.day = Some(read_digits(&text, &mut at, 2)?.0),
            Tok::DayOfYear(_) => f.doy = Some(read_digits(&text, &mut at, 3)?.0),
            Tok::Weekday(n) if *n <= 2 => {
                read_digits(&text, &mut at, 1)?;
            }
            // A weekday that disagrees with the date is ignored on input.
            Tok::Weekday(_) => {
                read_name(&text, &mut at, &DAYS)?;
            }
            Tok::Hour24(_) => f.hour = Some(read_digits(&text, &mut at, 2)?.0),
            Tok::Hour12(_) => {
                f.hour = Some(read_digits(&text, &mut at, 2)?.0);
                f.hour12 = true;
            }
            Tok::Minute(_) => f.minute = read_digits(&text, &mut at, 2)?.0,
            Tok::Second(_) => f.second = read_digits(&text, &mut at, 2)?.0,
            Tok::Frac(n) => {
                let max = if adjacent { *n } else { 9 };
                let (v, count) = read_digits(&text, &mut at, max)?;
                f.frac = v * 10i64.pow(9 - count as u32);
            }
            Tok::AmPm => {
                let rest: String = text[at..].iter().take(2).collect::<String>().to_uppercase();
                f.pm = Some(match rest.as_str() {
                    "AM" => false,
                    "PM" => true,
                    _ => return None,
                });
                at += 2;
            }
        }
    }
    if at != text.len() {
        return None;
    }
    let y = f.year.unwrap_or(current_year);
    let mut hour = f.hour.unwrap_or(0);
    if f.hour12 || f.pm.is_some() {
        if f.hour.is_some() && !(1..=12).contains(&hour) {
            return None;
        }
        hour %= 12;
        if f.pm == Some(true) {
            hour += 12;
        }
    } else if !(0..=23).contains(&hour) {
        return None;
    }
    if f.minute > 59 || f.second > 59 {
        return None;
    }
    let (month, day) = match (f.month, f.day, f.doy) {
        (None, None, Some(doy)) => {
            if !(1..=if calendar::is_leap(y) { 366 } else { 365 }).contains(&doy) {
                return None;
            }
            (1, doy)
        }
        (m, d, _) => {
            let m = m.unwrap_or(1);
            let d = d.unwrap_or(1);
            if !(1..=12).contains(&m) || d < 1 || d > i64::from(month_length(y, m as usize)) {
                return None;
            }
            (m, d)
        }
    };
    let ns = ((hour * 60 + f.minute) * 60 + f.second) as i128 * 1_000_000_000 + i128::from(f.frac);
    calendar::from_fields(i128::from(y), i128::from(month), i128::from(day), ns).ok()
}

/// Formats tried, in order, for text without an InputFormat; the flag marks
/// date-only forms.
pub(crate) const AUTO_FORMATS: &[(&str, bool)] = &[
    ("yyyy-MM-dd'T'HH:mm:ss.SSSSSSSSS", false),
    ("yyyy-MM-dd'T'HH:mm:ss", false),
    ("yyyy-MM-dd HH:mm:ss.SSSSSSSSS", false),
    ("yyyy-MM-dd HH:mm:ss", false),
    ("yyyy-MM-dd HH:mm", false),
    ("yyyy-MM-dd", true),
    ("dd-MMM-yyyy HH:mm:ss.SSSSSSSSS", false),
    ("dd-MMM-yyyy HH:mm:ss", false),
    ("dd-MMM-yyyy HH:mm", false),
    ("dd-MMM-yyyy", true),
    ("MM/dd/yyyy HH:mm:ss", false),
    ("MM/dd/yyyy", true),
];

/// Value of `text` in any automatic format, with whether the form was
/// date-only.
pub(crate) fn parse_auto(
    text: &str,
    pivot: i64,
    current_year: i64,
) -> Option<(DateTimeValue, bool)> {
    parse_auto_form(text, pivot, current_year).map(|(v, date_only, _)| (v, date_only))
}

/// [`parse_auto`] with the automatic format that matched.
pub(crate) fn parse_auto_form(
    text: &str,
    pivot: i64,
    current_year: i64,
) -> Option<(DateTimeValue, bool, &'static str)> {
    for (format, date_only) in AUTO_FORMATS {
        let tokens = tokens(format).ok()?;
        if let Some(v) = parse(&tokens, text, pivot, current_year) {
            return Some((v, *date_only, format));
        }
    }
    None
}

/// Whether a format has a time-of-day field (text read with a format
/// without one holds dates only).
pub(crate) fn has_time_fields(tokens: &[Tok]) -> bool {
    tokens.iter().any(|t| {
        matches!(
            t,
            Tok::Hour24(_)
                | Tok::Hour12(_)
                | Tok::Minute(_)
                | Tok::Second(_)
                | Tok::Frac(_)
                | Tok::AmPm
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(format: &str, text: &str) -> String {
        let t = tokens(format).unwrap();
        render(&t, parse(&t, text, 1976, 2026).unwrap())
    }

    #[test]
    fn letters_round_trip() {
        assert_eq!(
            round(
                "yyyy-MM-dd HH:mm:ss.SSSSSSSSS",
                "1969-12-31 23:59:59.123456789"
            ),
            "1969-12-31 23:59:59.123456789"
        );
        assert_eq!(round("dd-MMM-yyyy", "05-Mar-2024"), "05-Mar-2024");
        assert_eq!(
            round("eeee, MMMM d, yyyy h:mm a", "Friday, March 1, 2024 1:05 PM"),
            "Friday, March 1, 2024 1:05 PM"
        );
        assert_eq!(
            round("yyyy-MM-dd'T'HH:mm:ss", "2024-02-29T12:00:00"),
            "2024-02-29T12:00:00"
        );
        assert_eq!(round("yyyyMMdd", "20240229"), "20240229");
        assert_eq!(round("yyyy DDD", "2024 366"), "2024 366");
    }

    #[test]
    fn invalid_dates_do_not_parse() {
        let t = tokens("yyyy-MM-dd").unwrap();
        assert!(parse(&t, "2023-02-29", 1976, 2026).is_none());
        assert!(parse(&t, "2023-13-01", 1976, 2026).is_none());
        assert!(tokens("yyyy-QQ").is_err());
    }
}
