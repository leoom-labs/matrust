//! duration display formats: digital 'dd:hh:mm:ss', 'hh:mm:ss', 'mm:ss',
//! 'hh:mm' (each with an optional '.S' .. '.SSSSSSSSS') and the unit forms
//! 's', 'm', 'h', 'd', 'y'.
use crate::Result;

/// Default duration display format.
pub(crate) const DEFAULT: &str = crate::duration_array::DEFAULT_FORMAT;

const MS_PER: [(char, f64); 5] = [
    ('s', 1000.0),
    ('m', 60_000.0),
    ('h', 3_600_000.0),
    ('d', 86_400_000.0),
    ('y', 86_400_000.0 * 365.2425),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DurFormat {
    /// Leading field (0 days, 1 hours, 2 minutes), field count, fraction digits.
    Digital {
        lead: usize,
        fields: usize,
        frac: usize,
    },
    Unit(char),
}

pub(crate) fn unit_ms(unit: char) -> f64 {
    MS_PER
        .iter()
        .find(|(c, _)| *c == unit)
        .map_or(1.0, |(_, v)| *v)
}

pub(crate) fn parse_format(format: &str) -> Result<DurFormat> {
    if let [c] = format.chars().collect::<Vec<_>>().as_slice()
        && MS_PER.iter().any(|(u, _)| u == c)
    {
        return Ok(DurFormat::Unit(*c));
    }
    let (main, frac) = match format.split_once('.') {
        Some((main, s)) if !s.is_empty() && s.len() <= 9 && s.chars().all(|c| c == 'S') => {
            (main, s.len())
        }
        Some(_) => return Err(format!("Unsupported duration format '{format}'")),
        None => (format, 0),
    };
    let (lead, fields) = match main {
        "dd:hh:mm:ss" => (0, 4),
        "hh:mm:ss" => (1, 3),
        "mm:ss" => (2, 2),
        "hh:mm" if frac == 0 => (1, 2),
        _ => return Err(format!("Unsupported duration format '{format}'")),
    };
    Ok(DurFormat::Digital { lead, fields, frac })
}

/// Format for reading duration text. The unit forms ('s', 'm', 'h', 'd', 'y')
/// only print, and an unknown letter is not a duration format letter: both
/// are errors that name the letter, never a silent NaN.
pub(crate) fn parse_input_format(format: &str) -> Result<DurFormat> {
    match parse_format(format) {
        Ok(DurFormat::Unit(u)) => Err(format!(
            "duration: the unit letter '{u}' cannot be used in an InputFormat; use 'hh:mm:ss', 'dd:hh:mm:ss', 'mm:ss' or 'hh:mm'"
        )),
        Ok(f) => Ok(f),
        Err(e) => match format
            .chars()
            .find(|c| !matches!(c, 'd' | 'h' | 'm' | 's' | 'S' | ':' | '.'))
        {
            Some(c) => Err(format!(
                "duration: unsupported letter '{c}' in InputFormat '{format}'"
            )),
            None => Err(e),
        },
    }
}

/// Short-g text of a number (five significant digits), MATLAB style exponent.
pub(crate) fn short_g(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Inf".into() } else { "-Inf".into() };
    }
    if x == 0.0 {
        return "0".into();
    }
    if x.fract() == 0.0 && x.abs() < 1e10 {
        return format!("{x:.0}");
    }
    let exp = x.abs().log10().floor() as i32;
    if (-5..5).contains(&exp) {
        let decimals = (4 - exp).max(0) as usize;
        let s = format!("{x:.decimals$}");
        let s = if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        };
        return s;
    }
    let s = format!("{x:.4e}");
    let (mantissa, exponent) = s.split_once('e').unwrap_or((&s, "0"));
    let mantissa = if mantissa.contains('.') {
        mantissa.trim_end_matches('0').trim_end_matches('.')
    } else {
        mantissa
    };
    let e: i32 = exponent.parse().unwrap_or(0);
    format!("{mantissa}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
}

/// One ulp of `|x|`. `frexp` returns a mantissa in `[0.5, 1)`.
fn spacing(x: f64) -> f64 {
    let x = x.abs();
    if !x.is_finite() || x == 0. {
        return x;
    }
    let (_mant, exp) = libm::frexp(x);
    libm::ldexp(1.0, exp - 53)
}

/// Text of `ms` milliseconds in `format`.
pub(crate) fn render(format: DurFormat, ms: f64) -> String {
    match format {
        DurFormat::Unit(u) => {
            let v = ms / unit_ms(u);
            let plural = v != 1.0 && !v.is_nan();
            let suffix = match u {
                's' => "sec",
                'm' => "min",
                'h' => "hr",
                'd' if plural => "days",
                'd' => "day",
                'y' if plural => "yrs",
                _ => "yr",
            };
            format!("{} {suffix}", short_g(v))
        }
        DurFormat::Digital { lead, fields, frac } => {
            if ms.is_nan() {
                return "NaN".into();
            }
            if ms.is_infinite() {
                return if ms > 0.0 {
                    "Inf".into()
                } else {
                    "-Inf".into()
                };
            }
            let negative = ms < 0.0;
            let a = ms.abs();
            let mut out = String::new();
            if negative {
                out.push('-');
            }
            // Leading-field units in milliseconds: day, hour, minute.
            let lead_ms = [86_400_000.0, 3_600_000.0, 60_000.0][lead];
            // Last shown unit: one minute when seconds are absent, else
            // 10^(9-frac) nanoseconds. Round half away from zero (the sign
            // was stripped) so 60 carries into the next field.
            let shows_seconds = lead + fields > 3;
            let show_ns = if shows_seconds {
                10i128.pow(9 - frac as u32)
            } else {
                60 * 1_000_000_000
            };
            let show_ms = show_ns as f64 / 1e6;
            if a * 1e6 >= 1e36 {
                // Beyond exact integer nanoseconds: only the leading field.
                // Rounding through a finer unit overflows once ulp(a) already
                // exceeds that unit (milliseconds(1e307) became Inf).
                let rounded = if spacing(a) > show_ms {
                    a
                } else {
                    (a / show_ms).round() * show_ms
                };
                out.push_str(&format!("{:.0}", (rounded / lead_ms).floor()));
                return out;
            }
            let ns = (a * 1e6).round() as i128;
            let q = ns / show_ns;
            let r = ns % show_ns;
            let ns = (if r * 2 >= show_ns { q + 1 } else { q }) * show_ns;
            let unit = 10i128.pow(9 - frac as u32);
            let ticks = ns / unit;
            let per_second = 10i128.pow(frac as u32);
            let total_seconds = ticks / per_second;
            let sub = ticks % per_second;
            let sizes = [86_400i128, 3600, 60, 1];
            let mut rest = total_seconds;
            for (k, &size) in sizes.iter().enumerate().skip(lead).take(fields) {
                let last = k + 1 == lead + fields;
                let value = rest / size;
                rest -= value * size;
                if k > lead {
                    out.push(':');
                }
                out.push_str(&format!("{value:02}"));
                if last && frac > 0 && k == 3 {
                    out.push('.');
                    out.push_str(&format!("{sub:0frac$}"));
                }
            }
            out
        }
    }
}

/// Milliseconds of duration text in `format` (digital forms only); None when
/// the text does not match.
pub(crate) fn parse(format: DurFormat, text: &str) -> Option<f64> {
    let DurFormat::Digital { lead, fields, .. } = format else {
        return None;
    };
    let text = text.trim();
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let parts: Vec<&str> = body.split(':').collect();
    if parts.len() != fields {
        return None;
    }
    let sizes: [i128; 4] = [86_400, 3600, 60, 1];
    let limits: [i128; 4] = [i128::MAX, 24, 60, 60];
    let mut ns: i128 = 0;
    for (i, part) in parts.iter().enumerate() {
        let k = lead + i;
        let last = i + 1 == parts.len();
        let (whole, frac) = match part.split_once('.') {
            Some((w, f)) if last && k == 3 => (w, f),
            Some(_) => return None,
            None => (*part, ""),
        };
        if whole.is_empty()
            || whole.len() > 15
            || !whole.chars().all(|c| c.is_ascii_digit())
            || frac.len() > 9
            || !frac.chars().all(|c| c.is_ascii_digit())
        {
            return None;
        }
        let v: i128 = whole.parse().ok()?;
        if i > 0 && v >= limits[k] {
            return None;
        }
        ns += v * sizes[k] * 1_000_000_000;
        if !frac.is_empty() {
            let f: i128 = frac.parse().ok()?;
            ns += f * 10i128.pow(9 - frac.len() as u32);
        }
    }
    let ms = super::calendar::nanos_to_ms(ns);
    Some(if negative { -ms } else { ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digital_and_unit_text() {
        let f = parse_format("hh:mm:ss").unwrap();
        assert_eq!(render(f, 5_400_000.0), "01:30:00");
        assert_eq!(render(f, -5_400_000.0), "-01:30:00");
        assert_eq!(render(f, 3_600_000.0 * 123.0), "123:00:00");
        let f = parse_format("dd:hh:mm:ss.SSS").unwrap();
        assert_eq!(render(f, 90_061_001.0), "01:01:01:01.001");
        assert_eq!(render(parse_format("h").unwrap(), 5_400_000.0), "1.5 hr");
        assert_eq!(render(parse_format("d").unwrap(), 86_400_000.0), "1 day");
        assert_eq!(
            parse(parse_format("hh:mm:ss").unwrap(), "01:30:00.5"),
            Some(5_400_500.0)
        );
        assert_eq!(short_g(123456.0), "123456");
        assert_eq!(short_g(1.23456), "1.2346");
        assert_eq!(short_g(1.5e-7), "1.5e-07");
    }
}
