//! Datetime, duration, and categorical axis coordinate, tick, and label helpers.

use crate::datetime_array::{DEFAULT_DATE_FORMAT, DateTimeValue, NANOS_PER_DAY};
use crate::engine::datetime_fns::{calendar, dur_text, text};

/// Fixed serial-day epoch (1970-01-01) for datetime axis coordinates.
pub const EPOCH_DAYS: i64 = 719_529;

pub fn datetime_to_plot(v: DateTimeValue) -> f64 {
    if v.is_nat() {
        f64::NAN
    } else {
        (v.days - EPOCH_DAYS) as f64 + (v.nanos as f64) / (NANOS_PER_DAY as f64)
    }
}

pub fn plot_to_datetime(d: f64) -> DateTimeValue {
    if !d.is_finite() {
        return DateTimeValue::NAT;
    }
    let whole = d.floor();
    let frac = d - whole;
    let mut nanos = (frac * (NANOS_PER_DAY as f64)).round() as i64;
    let mut days = EPOCH_DAYS.saturating_add(whole as i64);
    if nanos >= NANOS_PER_DAY {
        nanos -= NANOS_PER_DAY;
        days = days.saturating_add(1);
    } else if nanos < 0 {
        nanos += NANOS_PER_DAY;
        days = days.saturating_sub(1);
    }
    DateTimeValue { days, nanos }
}

pub fn duration_unit_ms(format: &str) -> f64 {
    match dur_text::parse_format(format) {
        Ok(dur_text::DurFormat::Unit(u)) => dur_text::unit_ms(u),
        _ => 1_000.0,
    }
}

/// Nice calendar ticks for a datetime axis range in plot days.
/// Returns `(ticks, step_days, default_format)`.
pub fn datetime_ticks(range: [f64; 2]) -> (Vec<f64>, f64, &'static str) {
    let [lo, hi] = range;
    if !(lo.is_finite() && hi.is_finite() && lo < hi) {
        let t = if lo.is_finite() { vec![lo] } else { Vec::new() };
        return (t, 1.0, DEFAULT_DATE_FORMAT);
    }
    let span = hi - lo;
    let start_dt = plot_to_datetime(lo);
    let end_dt = plot_to_datetime(hi);
    let sp = calendar::parts(start_dt);
    let ep = calendar::parts(end_dt);

    if span >= 540.0 {
        let years_span = span / 365.2425;
        let step_y = [1_i64, 2, 5, 10, 20, 50, 100, 200, 500, 1000]
            .into_iter()
            .find(|&s| years_span / (s as f64) <= 9.0)
            .unwrap_or(1000);
        let after_jan1 = sp.mo > 1 || sp.d > 1 || start_dt.nanos > 0;
        let mut y = sp.y + i64::from(after_jan1);
        y = y.div_euclid(step_y) * step_y + if y.rem_euclid(step_y) != 0 { step_y } else { 0 };
        let mut out = Vec::new();
        while y <= ep.y && out.len() < 32 {
            if let Ok(dt) = calendar::from_fields(i128::from(y), 1, 1, 0) {
                let p = datetime_to_plot(dt);
                if p >= lo - 1e-9 && p <= hi + 1e-9 {
                    out.push(p);
                }
            }
            y += step_y;
        }
        return (out, (step_y as f64) * 365.2425, "yyyy");
    }

    if span >= 45.0 {
        let months_span = span / 30.4375;
        let step_m = [1_i64, 2, 3, 6]
            .into_iter()
            .find(|&s| months_span / (s as f64) <= 9.0)
            .unwrap_or(6);
        let after_day1 = sp.d > 1 || start_dt.nanos > 0;
        let mut total_m = sp.y * 12 + (sp.mo as i64 - 1) + i64::from(after_day1);
        let rem = total_m.rem_euclid(step_m);
        if rem != 0 {
            total_m += step_m - rem;
        }
        let end_m = ep.y * 12 + (ep.mo as i64 - 1);
        let mut out = Vec::new();
        while total_m <= end_m && out.len() < 32 {
            let y = total_m.div_euclid(12);
            let mo = total_m.rem_euclid(12) + 1;
            if let Ok(dt) = calendar::from_fields(i128::from(y), i128::from(mo), 1, 0) {
                let p = datetime_to_plot(dt);
                if p >= lo - 1e-9 && p <= hi + 1e-9 {
                    out.push(p);
                }
            }
            total_m += step_m;
        }
        return (out, (step_m as f64) * 30.4375, "MMM yyyy");
    }

    if span >= 1.5 {
        let step_d = [1_i64, 2, 3, 5, 7, 10, 14]
            .into_iter()
            .find(|&s| span / (s as f64) <= 9.0)
            .unwrap_or(14);
        let mut day = start_dt.days + i64::from(start_dt.nanos > 0);
        let mut out = Vec::new();
        while day <= end_dt.days && out.len() < 32 {
            let p = datetime_to_plot(DateTimeValue {
                days: day,
                nanos: 0,
            });
            if p >= lo - 1e-9 && p <= hi + 1e-9 {
                out.push(p);
            }
            day += step_d;
        }
        return (out, step_d as f64, DEFAULT_DATE_FORMAT);
    }

    if span >= 2.0 / 24.0 {
        let hours_span = span * 24.0;
        let step_h = [1_i64, 2, 3, 4, 6, 12]
            .into_iter()
            .find(|&s| hours_span / (s as f64) <= 9.0)
            .unwrap_or(12);
        return subday_ticks(lo, hi, (step_h as f64) / 24.0, "HH:mm");
    }

    if span >= 2.0 / 1440.0 {
        let mins_span = span * 1440.0;
        let step_m = [1_i64, 2, 5, 10, 15, 30]
            .into_iter()
            .find(|&s| mins_span / (s as f64) <= 9.0)
            .unwrap_or(30);
        return subday_ticks(lo, hi, (step_m as f64) / 1440.0, "HH:mm");
    }

    let secs_span = span * 86_400.0;
    let step_s = nice_sub_step(secs_span / 6.0);
    let fmt = if step_s < 1.0 {
        "HH:mm:ss.SSS"
    } else {
        "HH:mm:ss"
    };
    subday_ticks(lo, hi, step_s / 86_400.0, fmt)
}

fn subday_ticks(lo: f64, hi: f64, step: f64, fmt: &'static str) -> (Vec<f64>, f64, &'static str) {
    let first = (lo / step - 1e-9).ceil() * step;
    let mut out = Vec::new();
    let mut k = 0;
    loop {
        let v = first + (k as f64) * step;
        if v > hi + step * 1e-6 || out.len() >= 32 {
            break;
        }
        out.push(v);
        k += 1;
    }
    (out, step, fmt)
}

fn nice_sub_step(raw: f64) -> f64 {
    if !(raw.is_finite() && raw > 0.0) {
        return 1.0;
    }
    if raw >= 1.0 {
        for s in [1.0, 2.0, 5.0, 10.0, 15.0, 30.0] {
            if raw <= s {
                return s;
            }
        }
        return 30.0;
    }
    let exp = raw.log10().floor();
    let base = 10_f64.powf(exp);
    let frac = raw / base;
    let nice = if frac <= 1.0 {
        1.0
    } else if frac <= 2.0 {
        2.0
    } else if frac <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * base
}

pub fn secondary_date_label(range: [f64; 2]) -> Option<String> {
    let [lo, hi] = range;
    if lo.is_finite() && hi.is_finite() && lo < hi && (hi - lo) < 1.0 {
        Some(format_datetime_tick(lo, DEFAULT_DATE_FORMAT))
    } else {
        None
    }
}

pub fn format_datetime_tick(d: f64, format: &str) -> String {
    let mut dt = plot_to_datetime(d);
    if dt.is_nat() {
        return "NaT".into();
    }
    // A day count near 20000 carries ~0.4 ms of f64 error, so a 02:00 tick can
    // arrive as 01:59:59.9996 and print "01:59"; ticks sit on whole milliseconds.
    const MS: i64 = 1_000_000;
    dt.nanos = (dt.nanos + MS / 2) / MS * MS;
    if dt.nanos >= NANOS_PER_DAY {
        dt.nanos -= NANOS_PER_DAY;
        dt.days = dt.days.saturating_add(1);
    }
    let fmt = if format.is_empty() {
        DEFAULT_DATE_FORMAT
    } else {
        format
    };
    match text::tokens(fmt) {
        Ok(toks) => text::render(&toks, dt),
        Err(_) => {
            let toks = text::tokens(DEFAULT_DATE_FORMAT).unwrap_or_default();
            text::render(&toks, dt)
        }
    }
}

/// Nice ticks for a duration axis in the ruler's internal unit (`unit_ms` ms per 1.0).
pub fn duration_ticks(range: [f64; 2], unit_ms: f64, format: &str) -> (Vec<f64>, f64) {
    let [lo, hi] = range;
    if !(lo.is_finite() && hi.is_finite() && lo < hi) {
        let t = if lo.is_finite() { vec![lo] } else { Vec::new() };
        return (t, 1.0);
    }
    match dur_text::parse_format(format) {
        Ok(dur_text::DurFormat::Unit(u)) => {
            let factor = unit_ms / dur_text::unit_ms(u);
            let (scaled_ticks, scaled_step) = super::svg::ticks([lo * factor, hi * factor]);
            (
                scaled_ticks.into_iter().map(|v| v / factor).collect(),
                scaled_step / factor,
            )
        }
        _ => {
            // Digital format: choose nice multiples in seconds, minutes, hours, or days.
            let sec_per_unit = unit_ms / 1_000.0;
            let lo_s = lo * sec_per_unit;
            let hi_s = hi * sec_per_unit;
            let span_s = hi_s - lo_s;
            let step_s = if span_s >= 86_400.0 * 2.0 {
                let d = nice_sub_step(span_s / 86_400.0 / 6.0).max(1.0);
                d * 86_400.0
            } else if span_s >= 7_200.0 {
                let h = [1.0, 2.0, 3.0, 4.0, 6.0, 12.0, 24.0]
                    .into_iter()
                    .find(|&s| (span_s / 3_600.0) / s <= 9.0)
                    .unwrap_or(24.0);
                h * 3_600.0
            } else if span_s >= 120.0 {
                let m = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0]
                    .into_iter()
                    .find(|&s| (span_s / 60.0) / s <= 9.0)
                    .unwrap_or(30.0);
                m * 60.0
            } else {
                nice_sub_step(span_s / 6.0)
            };
            let first = (lo_s / step_s - 1e-9).ceil() * step_s;
            let mut ticks = Vec::new();
            let mut k = 0;
            loop {
                let s = first + (k as f64) * step_s;
                if s > hi_s + step_s * 1e-6 || ticks.len() >= 32 {
                    break;
                }
                ticks.push(s / sec_per_unit);
                k += 1;
            }
            (ticks, step_s / sec_per_unit)
        }
    }
}

pub fn format_duration_tick(v: f64, unit_ms: f64, format: &str) -> String {
    let fmt = dur_text::parse_format(format).unwrap_or(dur_text::DurFormat::Digital {
        lead: 1,
        fields: 3,
        frac: 0,
    });
    dur_text::render(fmt, v * unit_ms)
}
