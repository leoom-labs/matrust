//! Integer calendar arithmetic for datetime values. Day numbers are the
//! serial days of `datenum` (dates.rs `civil`/`civil_serial`); every carry is
//! done in i128 so no input can wrap or panic.
use super::super::dates::{UNIX_DAY, civil, civil_serial};
use crate::Result;
use crate::datetime_array::{DateTimeValue, NANOS_PER_DAY, TimeZone};

const NPD: i128 = NANOS_PER_DAY as i128;
/// Years beyond +-1e8 are rejected (the calendar math stays far inside i64).
const MAX_YEAR: i128 = 100_000_000;
/// Day numbers inside +-MAX_YEAR years.
const MAX_DAYS: i128 = 36_524_250_000;
/// Components at or beyond this magnitude are rejected, like dates.rs.
const MAX_COMPONENT: f64 = 1e15;
pub(super) const OUT_OF_RANGE: &str =
    "datetime: date is out of range (supported years: -100000000 to 100000000)";

/// Calendar fields of one value.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Parts {
    pub y: i64,
    pub mo: usize,
    pub d: u32,
    pub h: u32,
    pub mi: u32,
    pub s: u32,
    /// Nanoseconds within the second.
    pub ns: u32,
    /// 1 = Sunday .. 7 = Saturday.
    pub weekday: usize,
    pub doy: u32,
}

pub(crate) fn parts(v: DateTimeValue) -> Parts {
    let (y, mo, d) = civil(v.days);
    let n = v.nanos;
    let secs = n / 1_000_000_000;
    Parts {
        y,
        mo,
        d,
        h: (secs / 3600) as u32,
        mi: (secs / 60 % 60) as u32,
        s: (secs % 60) as u32,
        ns: (n % 1_000_000_000) as u32,
        weekday: weekday(v.days),
        doy: (v.days - civil_serial(y, 1, 1) + 1) as u32,
    }
}

pub(super) fn weekday(days: i64) -> usize {
    (days + 5).rem_euclid(7) as usize + 1
}

pub(super) fn is_leap(y: i64) -> bool {
    y.rem_euclid(4) == 0 && (y.rem_euclid(100) != 0 || y.rem_euclid(400) == 0)
}

pub(super) fn month_length(y: i64, m: usize) -> u32 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Value of year `y`, month `mo` (any integer, carried into the year), day
/// `d` (any integer, carried) and `ns` nanoseconds after that day's midnight
/// (any sign, carried into days).
pub(crate) fn from_fields(y: i128, mo: i128, d: i128, ns: i128) -> Result<DateTimeValue> {
    let y = y + (mo - 1).div_euclid(12);
    let m = (mo - 1).rem_euclid(12) as usize + 1;
    if y.abs() > MAX_YEAR || d.abs() > MAX_DAYS * 2 {
        return Err(OUT_OF_RANGE.into());
    }
    let first = i128::from(civil_serial(y as i64, m, 1));
    let days = first + d - 1 + ns.div_euclid(NPD);
    if days.abs() > MAX_DAYS {
        return Err(OUT_OF_RANGE.into());
    }
    Ok(DateTimeValue {
        days: days as i64,
        nanos: ns.rem_euclid(NPD) as i64,
    })
}

/// Value from a total nanosecond count since the serial epoch.
pub(super) fn from_total(total: i128) -> Result<DateTimeValue> {
    let days = total.div_euclid(NPD);
    if days.abs() > MAX_DAYS {
        return Err(OUT_OF_RANGE.into());
    }
    Ok(DateTimeValue {
        days: days as i64,
        nanos: total.rem_euclid(NPD) as i64,
    })
}

/// `x` units of `unit_ns` nanoseconds, as exact integer nanoseconds: the
/// whole part multiplies exactly and only the fraction is rounded.
pub(super) fn scaled_nanos(x: f64, unit_ns: i128) -> Result<i128> {
    if x.abs() >= MAX_COMPONENT {
        return Err("datetime: components must be below 1e15 in magnitude".into());
    }
    let whole = x.trunc();
    let frac = x - whole;
    Ok(whole as i128 * unit_ns + (frac * unit_ns as f64).round() as i128)
}

/// datetime(Y, M, D, H, MI, S, MS): NaN or Inf anywhere gives NaT. Year and
/// month must be integers; the other parts may hold fractions and carry.
pub(super) fn from_components(c: [f64; 7]) -> Result<DateTimeValue> {
    if c.iter().any(|v| !v.is_finite()) {
        return Ok(DateTimeValue::NAT);
    }
    let [y, mo, d, h, mi, s, ms] = c;
    if mo.abs() >= MAX_COMPONENT || y.abs() >= MAX_COMPONENT {
        return Err("datetime: year and month must be below 1e15 in magnitude".into());
    }
    if y.fract() != 0.0 || mo.fract() != 0.0 {
        return Err("datetime: year and month must be integers".into());
    }
    let whole_d = d.trunc();
    let mut ns = scaled_nanos(d - whole_d, NPD)?;
    ns += scaled_nanos(h, 3_600_000_000_000)?;
    ns += scaled_nanos(mi, 60_000_000_000)?;
    ns += scaled_nanos(s, 1_000_000_000)?;
    ns += scaled_nanos(ms, 1_000_000)?;
    if whole_d.abs() >= MAX_COMPONENT {
        return Err("datetime: components must be below 1e15 in magnitude".into());
    }
    from_fields(y as i128, mo as i128, whole_d as i128, ns)
}

/// Days plus a fraction of a day (`datenum`-like numbers) on a given epoch
/// day, the fraction rounded to `resolution_ns`.
pub(super) fn from_day_number(x: f64, epoch: i64, resolution_ns: i128) -> Result<DateTimeValue> {
    if !x.is_finite() {
        return Ok(DateTimeValue::NAT);
    }
    if x.abs() >= MAX_COMPONENT {
        return Err(OUT_OF_RANGE.into());
    }
    let whole = x.floor();
    let frac = x - whole;
    let units = (frac * (NPD / resolution_ns) as f64).round() as i128;
    from_total((whole as i128 + i128::from(epoch)) * NPD + units * resolution_ns)
}

pub(super) fn to_datenum(v: DateTimeValue) -> f64 {
    if v.is_nat() {
        return f64::NAN;
    }
    v.days as f64 + v.nanos as f64 / NANOS_PER_DAY as f64
}

pub(super) fn to_posix(v: DateTimeValue) -> f64 {
    let Some(total) = v.total_nanos() else {
        return f64::NAN;
    };
    let since = total - i128::from(UNIX_DAY) * NPD;
    let secs = since.div_euclid(1_000_000_000);
    let frac = since.rem_euclid(1_000_000_000);
    secs as f64 + frac as f64 / 1e9
}

pub(super) fn from_posix(x: f64) -> Result<DateTimeValue> {
    if !x.is_finite() {
        return Ok(DateTimeValue::NAT);
    }
    if x.abs() >= 1e18 {
        return Err(OUT_OF_RANGE.into());
    }
    let whole = x.floor();
    let ns = whole as i128 * 1_000_000_000 + ((x - whole) * 1e9).round() as i128;
    from_total(i128::from(UNIX_DAY) * NPD + ns)
}

/// Julian date: datenum + 1721058.5.
pub(super) fn to_julian(v: DateTimeValue) -> f64 {
    if v.is_nat() {
        return f64::NAN;
    }
    // Shift by half a day in integers first, then add the fraction once.
    let total = i128::from(v.days + 1_721_058) * NPD + i128::from(v.nanos) + NPD / 2;
    let days = total.div_euclid(NPD);
    days as f64 + total.rem_euclid(NPD) as f64 / NPD as f64
}

pub(super) fn from_julian(x: f64) -> Result<DateTimeValue> {
    if !x.is_finite() {
        return Ok(DateTimeValue::NAT);
    }
    if x.abs() >= MAX_COMPONENT {
        return Err(OUT_OF_RANGE.into());
    }
    let whole = x.floor();
    let ns = ((x - whole) * NPD as f64).round() as i128;
    from_total((whole as i128 - 1_721_059) * NPD + ns + NPD / 2)
}

/// Serial day of 31-Dec-1899 (Excel day 0 for serials below 60).
fn excel_base() -> i64 {
    civil_serial(1899, 12, 31)
}

/// Excel serial (1900 system, with its leap-year hole) to a value. Serial 60
/// is 29-Feb-1900, which does not exist: NaT. Rounded to the microsecond.
pub(super) fn from_excel(x: f64, system_1904: bool) -> Result<DateTimeValue> {
    if !x.is_finite() {
        return Ok(DateTimeValue::NAT);
    }
    if system_1904 {
        return from_day_number(x, civil_serial(1904, 1, 1), 1000);
    }
    if (60.0..61.0).contains(&x) {
        return Ok(DateTimeValue::NAT);
    }
    let base = if x >= 61.0 {
        excel_base() - 1
    } else {
        excel_base()
    };
    from_day_number(x, base, 1000)
}

pub(super) fn to_excel(v: DateTimeValue, system_1904: bool) -> f64 {
    if v.is_nat() {
        return f64::NAN;
    }
    let base = if system_1904 {
        civil_serial(1904, 1, 1)
    } else if v.days >= civil_serial(1900, 3, 1) {
        excel_base() - 1
    } else {
        excel_base()
    };
    (v.days - base) as f64 + v.nanos as f64 / NANOS_PER_DAY as f64
}

/// yyyymmdd numbers: integer values only; anything else is NaT.
pub(super) fn from_yyyymmdd(x: f64) -> Result<DateTimeValue> {
    if !x.is_finite() || x.fract() != 0.0 || x.abs() >= MAX_COMPONENT {
        return Ok(DateTimeValue::NAT);
    }
    let n = x as i128;
    from_fields(
        n.div_euclid(10_000),
        n.div_euclid(100).rem_euclid(100),
        n.rem_euclid(100),
        0,
    )
}

/// The current date and time: local wall-clock components when unzoned, UTC
/// components for 'UTC'. The sub-second part comes from the system clock.
pub(super) fn now(zone: TimeZone) -> Result<DateTimeValue> {
    let time = crate::clock::system_now();
    let since = match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => i128::try_from(d.as_nanos()).map_err(|_| OUT_OF_RANGE)?,
        Err(e) => -i128::try_from(e.duration().as_nanos()).map_err(|_| OUT_OF_RANGE)?,
    };
    match zone {
        TimeZone::Utc => from_total(i128::from(UNIX_DAY) * NPD + since),
        TimeZone::Unzoned => {
            let p = super::super::env_files::envx_local_parts(time)?;
            let sub = since.rem_euclid(1_000_000_000);
            let ns = (p[3] as i128 * 3600 + p[4] as i128 * 60 + p[5].floor() as i128)
                * 1_000_000_000
                + sub;
            from_fields(p[0] as i128, p[1] as i128, p[2] as i128, ns)
        }
    }
}

/// `v` shifted by `ns` nanoseconds (NaT stays NaT).
pub(super) fn shifted(v: DateTimeValue, ns: Option<i128>) -> Result<DateTimeValue> {
    match (v.total_nanos(), ns) {
        (Some(t), Some(ns)) => from_total(t + ns),
        _ => Ok(DateTimeValue::NAT),
    }
}

/// Milliseconds of a nanosecond difference, rounded once.
pub(super) fn nanos_to_ms(ns: i128) -> f64 {
    let whole = ns.div_euclid(1_000_000);
    let rem = ns.rem_euclid(1_000_000);
    whole as f64 + rem as f64 / 1e6
}

/// Exact nanoseconds of `ms` milliseconds: None for NaN/Inf (a NaT
/// result), an out-of-range error when no datetime could hold the result.
pub(super) fn ms_to_nanos(ms: f64) -> Result<Option<i128>> {
    // +-1e8 years are about +-3.2e18 ms.
    shift_nanos(ms, 1_000_000, 1e19)
}

/// Exact nanoseconds of `days` days of 24 hours, like [`ms_to_nanos`].
pub(super) fn days_to_nanos(days: f64) -> Result<Option<i128>> {
    // +-1e8 years are about +-3.7e10 days.
    shift_nanos(days, NPD, 1e14)
}

/// `x` units of `unit_ns`: the whole part multiplies exactly and only the
/// fraction rounds (as [`scaled_nanos`], with a wider limit for shifts).
fn shift_nanos(x: f64, unit_ns: i128, limit: f64) -> Result<Option<i128>> {
    if !x.is_finite() {
        return Ok(None);
    }
    if x.abs() >= limit {
        return Err(OUT_OF_RANGE.into());
    }
    let whole = x.trunc();
    Ok(Some(
        whole as i128 * unit_ns + ((x - whole) * unit_ns as f64).round() as i128,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip_and_anchors() {
        assert_eq!(civil_serial(1970, 1, 1), UNIX_DAY);
        assert_eq!(civil_serial(0, 1, 1), 1);
        for day in [1i64, 59, 60, 61, 366, 693_962, 719_529, 738_000, 3_652_425] {
            let (y, m, d) = civil(day);
            assert_eq!(civil_serial(y, m, i64::from(d)), day);
        }
        assert_eq!(weekday(UNIX_DAY), 5);
    }

    #[test]
    fn huge_components_error_without_panicking() {
        assert!(from_components([2024.0, 1e15, 1.0, 0.0, 0.0, 0.0, 0.0]).is_err());
        assert!(from_components([2024.0, 1.0, 1.0, 9e14, 0.0, 0.0, 0.0]).is_err());
        assert!(from_fields(i128::from(i64::MAX), 1, 1, 0).is_err());
        assert!(
            from_components([f64::NAN, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0])
                .unwrap()
                .is_nat()
        );
    }
}
