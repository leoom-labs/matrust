//! Timetables use ordinary shared table variables and a typed RowTimes column.
//! Resampling is serial, bounded and cancellable; no worker pools are created.
use super::*;
use crate::DurationArray;
use crate::datetime_array::{DateTimeValue, MAX_ELEMENTS, NANOS_PER_DAY};

fn unsupported(who: &str, option: &str) -> String {
    format!("{who}: {option} is not supported")
}

fn times(a: &Array, e: &Engine) -> Result<Vec<i128>> {
    match a {
        Array::Datetime(t) => t
            .data()
            .iter()
            .enumerate()
            .map(|(i, v)| {
                if i % 4096 == 0 {
                    e.check_cancel()?;
                }
                v.total_nanos()
                    .ok_or("timetable: NaT row times are not supported".into())
            })
            .collect(),
        Array::Duration(d) if !d.is_calendar() => d
            .data()
            .iter()
            .enumerate()
            .map(|(i, v)| {
                if i % 4096 == 0 {
                    e.check_cancel()?;
                }
                if !v.is_finite() || v.abs() >= 1e18 {
                    Err("timetable: finite row times below 1e18 milliseconds are required".into())
                } else {
                    Ok((v * 1e6).round() as i128)
                }
            })
            .collect(),
        _ => Err("timetable: row times must be datetime or duration values".into()),
    }
}

fn time_array(template: &Array, values: &[i128]) -> Result<Array> {
    match template {
        Array::Datetime(t) => Ok(t
            .with_data(
                values.len(),
                1,
                values
                    .iter()
                    .map(|&n| {
                        let days = n.div_euclid(NANOS_PER_DAY as i128);
                        if days.abs() > 36_524_250_000 {
                            return Err("timetable: date is out of range".into());
                        }
                        Ok(DateTimeValue {
                            days: days as i64,
                            nanos: n.rem_euclid(NANOS_PER_DAY as i128) as i64,
                        })
                    })
                    .collect::<Result<_>>()?,
            )?
            .into()),
        Array::Duration(d) => Ok(d
            .with_data(
                values.len(),
                1,
                values.iter().map(|&n| n as f64 / 1e6).collect(),
            )?
            .into()),
        _ => Err("timetable: invalid time type".into()),
    }
}

fn same_time_type(a: &Array, b: &Array) -> Result<()> {
    match (a, b) {
        (Array::Datetime(a), Array::Datetime(b)) if a.zone() == b.zone() => Ok(()),
        (Array::Duration(a), Array::Duration(b)) if !a.is_calendar() && !b.is_calendar() => Ok(()),
        _ => Err("timetable: row time types and time zones must match".into()),
    }
}

fn sorted(x: &[i128], strict: bool) -> Result<()> {
    if x.windows(2)
        .any(|w| w[0] > w[1] || (strict && w[0] == w[1]))
    {
        return Err("retime: row times must be sorted; interpolation requires unique times".into());
    }
    Ok(())
}

fn timetable<'a>(args: &'a [Value], who: &str) -> Result<&'a TableArray> {
    args.first()
        .and_then(table_of)
        .filter(|t| t.is_timetable())
        .ok_or_else(|| format!("{who}: expected a timetable"))
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    names: &[String],
    out: &mut Execution,
) -> Result<Value> {
    match name {
        "timetable" => construct(e, args, names),
        "table2timetable" => convert(e, args),
        "timetable2table" => {
            let t = timetable(args, name)?;
            if args.len() != 1 {
                return Err(unsupported(name, "options"));
            }
            let mut p = t.to_parts();
            let time = p.row_times.take().unwrap();
            let time_name = p.dimension_names[0].clone();
            p.dimension_names = ["Row".into(), "Variables".into()];
            p.names.insert(0, time_name);
            p.columns.insert(0, time);
            if !p.units.is_empty() {
                p.units.insert(0, String::new());
            }
            if !p.descriptions.is_empty() {
                p.descriptions.insert(0, String::new());
            }
            Ok(tvalue(TableArray::new(p)?))
        }
        "istimetable" => {
            if args.len() != 1 {
                return Err("istimetable: expected one input".into());
            }
            Ok(Value::Logical(LogicalMatrix::scalar(
                table_of(&args[0]).is_some_and(TableArray::is_timetable),
            )))
        }
        "retime" => {
            let t = timetable(args, name)?;
            if args.len() < 2 {
                return Err(unsupported(name, "this syntax or options"));
            }
            let mut constant = 0.0_f64;
            let (method, target, last_edge) = if text_scalar(&args[1]).as_deref() == Some("regular")
            {
                let rest = &args[2..];
                let (method, nv) = if rest.len() >= 2 && rest.len().is_multiple_of(2) {
                    ("fillwithmissing".to_string(), rest)
                } else if rest.len() >= 3 && !rest.len().is_multiple_of(2) {
                    (
                        text_scalar(&rest[0]).ok_or("retime: method must be text")?,
                        &rest[1..],
                    )
                } else {
                    return Err("retime: 'regular' requires 'TimeStep' or 'SampleRate'".into());
                };
                if !METHODS.contains(&method.as_str()) {
                    return Err(unsupported("retime", &format!("method '{method}'")));
                }
                let mut spec_pair: Option<(&Value, &Value)> = None;
                for pair in nv.chunks(2) {
                    let key = text_scalar(&pair[0]).ok_or("retime: option name must be text")?;
                    match key.to_ascii_lowercase().as_str() {
                        "timestep" | "samplerate" => {
                            if spec_pair.is_some() {
                                return Err(unsupported(
                                    "retime",
                                    "multiple TimeStep/SampleRate options",
                                ));
                            }
                            spec_pair = Some((&pair[0], &pair[1]));
                        }
                        "constant" => constant = parse_constant(e, &pair[1])?,
                        _ => return Err(unsupported("retime", &key)),
                    }
                }
                let (k_val, v_val) =
                    spec_pair.ok_or("retime: 'regular' requires 'TimeStep' or 'SampleRate'")?;
                let spec = parse_regular_spec(e, k_val, v_val, "retime")?;
                let template = t.row_times().unwrap();
                let x = times(template, e)?;
                sorted(&x, false)?;
                let (target, last_edge) =
                    if let (Some(&first_t), Some(&last_t)) = (x.first(), x.last()) {
                        regular_grid(e, template, first_t, last_t, spec, &method, "retime")?
                    } else {
                        (time_array(template, &[])?, None)
                    };
                (method, target, last_edge)
            } else {
                let rest = &args[2..];
                let (method, nv) = if rest.is_empty() {
                    ("fillwithmissing".to_string(), &[][..])
                } else if !rest.len().is_multiple_of(2) {
                    (
                        text_scalar(&rest[0]).ok_or("retime: method must be text")?,
                        &rest[1..],
                    )
                } else {
                    ("fillwithmissing".to_string(), rest)
                };
                if !METHODS.contains(&method.as_str()) {
                    return Err(unsupported("retime", &format!("method '{method}'")));
                }
                for pair in nv.chunks(2) {
                    let key = text_scalar(&pair[0]).ok_or("retime: option name must be text")?;
                    match key.to_ascii_lowercase().as_str() {
                        "constant" => constant = parse_constant(e, &pair[1])?,
                        _ => return Err(unsupported("retime", &key)),
                    }
                }
                let (target, last_edge) = target_times(e, t, &args[1], &method)?;
                (method, target, last_edge)
            };
            Ok(tvalue(resample_with_constant(
                e, t, &target, last_edge, &method, constant, out,
            )?))
        }
        "synchronize" => synchronize(e, args, out),
        _ => unreachable!(),
    }
}

fn construct(e: &mut Engine, args: &[Value], names: &[String]) -> Result<Value> {
    if args.is_empty() {
        let mut p = TableParts::new(Vec::new(), Vec::new(), 0);
        p.row_times = Some(DurationArray::new(0, 1, Vec::new(), "hh:mm:ss")?.into());
        p.dimension_names = ["Time".into(), "Variables".into()];
        return Ok(tvalue(TableArray::new(p)?));
    }
    // Only recognized name/value pairs at the tail are options. Scalar
    // strings (including a lone option-looking string) can be data variables.
    let mut option_start = args.len();
    while option_start >= 2 {
        let Some(key) = text_scalar(&args[option_start - 2]) else {
            break;
        };
        if ![
            "rowtimes",
            "samplerate",
            "timestep",
            "starttime",
            "variablenames",
            "dimensionnames",
        ]
        .contains(&key.to_ascii_lowercase().as_str())
        {
            break;
        }
        option_start -= 2;
    }
    let data = &args[..option_start];
    let opts = &args[option_start..];
    if !opts.len().is_multiple_of(2) {
        return Err(unsupported("timetable", "unpaired options"));
    }
    let mut rt = None;
    let mut step = None;
    let mut sample_rate = None;
    let mut calendar_step = None;
    let mut start = None;
    let mut var_names = None;
    let mut dims = None;
    for pair in opts.chunks(2) {
        let key = text_scalar(&pair[0]).ok_or("timetable: option name must be text")?;
        match key.to_ascii_lowercase().as_str() {
            "rowtimes" if rt.is_none() => rt = Some(pair[1].clone().array(&e.cancel)?),
            "samplerate" if step.is_none() && sample_rate.is_none() && calendar_step.is_none() => {
                let m = pair[1].clone().matrix(&e.cancel)?;
                if m.len() != 1 || !m.data()[0].is_finite() || m.data()[0] <= 0. {
                    return Err("timetable: SampleRate must be a positive finite scalar".into());
                }
                let fs = m.data()[0];
                let ns = 1e9 / fs;
                if !(1.0..1e24).contains(&ns) {
                    return Err("timetable: SampleRate produces an unsupported TimeStep".into());
                }
                // Row i is round(i*1e9/fs) from the start, not i*round(1e9/fs).
                sample_rate = Some(fs);
            }
            "timestep" if step.is_none() && sample_rate.is_none() && calendar_step.is_none() => {
                let a = pair[1].clone().array(&e.cancel)?;
                if !matches!(a, Array::Duration(_)) || a.len() != 1 {
                    return Err(unsupported(
                        "timetable",
                        "TimeStep other than a duration scalar",
                    ));
                }
                if matches!(&a,Array::Duration(d) if d.is_calendar()) {
                    calendar_step = Some(a);
                } else {
                    step = Some(times(&a, e)?[0]);
                }
            }
            "starttime" if start.is_none() => start = Some(pair[1].clone().array(&e.cancel)?),
            "variablenames" if var_names.is_none() => {
                var_names = Some(
                    text_list(e, &pair[1], "VariableNames")?
                        .ok_or("timetable: VariableNames must be text names")?,
                )
            }
            "dimensionnames" if dims.is_none() => {
                dims = Some(
                    text_list(e, &pair[1], "DimensionNames")?
                        .filter(|x| x.len() == 2)
                        .ok_or("timetable: DimensionNames must contain two names")?,
                )
            }
            _ => return Err(unsupported("timetable", &key)),
        }
    }
    let positional = rt.is_none()
        && step.is_none()
        && sample_rate.is_none()
        && calendar_step.is_none()
        && data.first().is_some_and(|v| {
            matches!(v, Value::Container(Array::Datetime(_) | Array::Duration(_)))
        });
    let offset = usize::from(positional);
    if positional {
        rt = Some(data[0].clone().array(&e.cancel)?);
    }
    if rt.is_some()
        && (step.is_some() || sample_rate.is_some() || calendar_step.is_some() || start.is_some())
    {
        return Err(
            "timetable: RowTimes cannot be combined with SampleRate, TimeStep or StartTime".into(),
        );
    }
    let columns = data[offset..]
        .iter()
        .map(|v| v.clone().array(&e.cancel))
        .collect::<Result<Vec<_>>>()?;
    let height = columns
        .first()
        .map(Array::rows)
        .or_else(|| rt.as_ref().map(Array::len))
        .unwrap_or(0);
    crate::datetime_array::checked_len(height, 1, "timetable")?;
    if rt.is_none() && calendar_step.is_some() {
        let template = start
            .take()
            .ok_or("timetable: calendar TimeStep requires a datetime StartTime")?;
        if !matches!(template, Array::Datetime(_)) || template.len() != 1 {
            return Err("timetable: calendar TimeStep requires a datetime StartTime".into());
        }
        let by = calendar_step.take().unwrap();
        let first = times(&template, e)?[0];
        let mut values = Vec::with_capacity(height);
        for i in 0..height.max(2) {
            if i % 4096 == 0 {
                e.check_cancel()?;
            }
            let scale = Value::Matrix(Matrix::scalar(i as f64));
            let delta = e.dtm_binary(
                &crate::syntax::Token::Star,
                &Value::from(by.clone()),
                &scale,
            )?;
            let value = e
                .dtm_binary(
                    &crate::syntax::Token::Plus,
                    &Value::from(template.clone()),
                    &delta,
                )?
                .array(&e.cancel)?;
            let n = times(&value, e)?[0];
            if i == 1 && n <= first {
                return Err("timetable: TimeStep must advance time".into());
            }
            if i < height {
                values.push(n);
            }
        }
        rt = Some(time_array(&template, &values)?);
    }
    if rt.is_none() {
        let step = if sample_rate.is_none() {
            let step = step.ok_or("timetable: provide RowTimes, SampleRate or TimeStep")?;
            if step <= 0 {
                return Err(
                    "timetable: TimeStep must be positive and at least one nanosecond".into(),
                );
            }
            Some(step)
        } else {
            None
        };
        crate::datetime_array::checked_len(height, 1, "timetable")?;
        let template = start.unwrap_or(Array::Duration(DurationArray::new(
            1,
            1,
            vec![0.],
            "hh:mm:ss",
        )?));
        if template.len() != 1 {
            return Err("timetable: StartTime must be scalar".into());
        }
        let first = times(&template, e)?[0];
        let mut values = Vec::with_capacity(height);
        for i in 0..height {
            if i % 4096 == 0 {
                e.check_cancel()?;
            }
            let delta = if let Some(fs) = sample_rate {
                let ticks = (i as f64) * 1e9 / fs;
                if !ticks.is_finite() {
                    return Err("timetable: SampleRate produces an unsupported TimeStep".into());
                }
                ticks.round() as i128
            } else {
                step.unwrap_or(0) * i as i128
            };
            values.push(first + delta);
        }
        rt = Some(time_array(&template, &values)?);
    }
    let rt = rt.unwrap();
    times(&rt, e)?;
    let ns = var_names.unwrap_or_else(|| {
        (0..columns.len())
            .map(|i| {
                names
                    .get(i + offset)
                    .filter(|s| valid_name(s))
                    .cloned()
                    .unwrap_or_else(|| format!("Var{}", i + 1))
            })
            .collect()
    });
    let mut p = TableParts::new(ns, columns, height);
    p.row_times = Some(rt);
    p.dimension_names = dims
        .map(|v| [v[0].clone(), v[1].clone()])
        .unwrap_or(["Time".into(), "Variables".into()]);
    Ok(tvalue(TableArray::new(p)?))
}

fn convert(e: &mut Engine, args: &[Value]) -> Result<Value> {
    let t = args
        .first()
        .and_then(table_of)
        .filter(|t| !t.is_timetable())
        .ok_or("table2timetable: expected a table")?;
    let mut p = t.to_parts();
    let times = if args.len() == 1 {
        let k = p
            .columns
            .iter()
            .position(|c| matches!(c, Array::Datetime(_) | Array::Duration(_)))
            .ok_or("table2timetable: no datetime or duration variable found")?;
        let a = p.columns[k].clone();
        let name = p.names[k].clone();
        p.remove_variables(&[k]);
        p.dimension_names[0] = name;
        a
    } else if args.len() == 3
        && text_scalar(&args[1]).is_some_and(|s| s.eq_ignore_ascii_case("RowTimes"))
    {
        let a = args[2].clone().array(&e.cancel)?;
        p.dimension_names[0] = "Time".into();
        a
    } else {
        return Err(unsupported("table2timetable", "these options"));
    };
    self::times(&times, e)?;
    p.row_times = Some(times);
    p.row_names = None;
    Ok(tvalue(TableArray::new(p)?))
}

// Calendar edges retain exact integer nanoseconds even at modern datenum epochs.
fn advance(n: i128, unit: &str) -> Result<i128> {
    let fixed = match unit {
        "secondly" => Some(1_000_000_000),
        "minutely" => Some(60_000_000_000),
        "hourly" => Some(3_600_000_000_000),
        "daily" => Some(NANOS_PER_DAY as i128),
        _ => None,
    };
    if let Some(step) = fixed {
        return Ok(n + step);
    }
    let day = n.div_euclid(NANOS_PER_DAY as i128) as i64;
    let (y, m, _) = super::super::dates::civil(day);
    let (y, m) = if unit == "monthly" {
        if m == 12 { (y + 1, 1) } else { (y, m + 1) }
    } else {
        (y + 1, 1)
    };
    Ok(super::super::dates::civil_serial(y, m, 1) as i128 * NANOS_PER_DAY as i128)
}

const MAX_TIME_ROWS: usize = (256 * 1024 * 1024) / 64;

enum RegularSpec {
    TimeStep(i128),
    SampleRate(f64),
    CalendarStep(Array),
}

fn is_aggregating(method: &str) -> bool {
    [
        "mean",
        "sum",
        "min",
        "max",
        "count",
        "firstvalue",
        "lastvalue",
    ]
    .contains(&method)
}

fn parse_regular_spec(e: &Engine, key_val: &Value, val: &Value, who: &str) -> Result<RegularSpec> {
    let key = text_scalar(key_val).ok_or_else(|| format!("{who}: option name must be text"))?;
    match key.to_ascii_lowercase().as_str() {
        "timestep" => {
            let a = val.clone().array(&e.cancel)?;
            if !matches!(a, Array::Duration(_)) || a.len() != 1 {
                return Err(unsupported(who, "TimeStep other than a duration scalar"));
            }
            if matches!(&a, Array::Duration(d) if d.is_calendar()) {
                Ok(RegularSpec::CalendarStep(a))
            } else {
                let step = times(&a, e)?[0];
                if step <= 0 {
                    return Err(format!("{who}: TimeStep must be positive"));
                }
                Ok(RegularSpec::TimeStep(step))
            }
        }
        "samplerate" => {
            let m = val.clone().matrix(&e.cancel)?;
            if m.len() != 1 || !m.data()[0].is_finite() || m.data()[0] <= 0. {
                return Err(format!(
                    "{who}: SampleRate must be a positive finite scalar"
                ));
            }
            let fs = m.data()[0];
            let ns = 1e9 / fs;
            if !(1.0..1e24).contains(&ns) {
                return Err(format!(
                    "{who}: SampleRate produces an unsupported TimeStep"
                ));
            }
            Ok(RegularSpec::SampleRate(fs))
        }
        _ => Err(unsupported(who, &key)),
    }
}

fn parse_constant(e: &Engine, v: &Value) -> Result<f64> {
    let a = v.clone().array(&e.cancel)?;
    if a.len() != 1 {
        return Err("retime: Constant must be a scalar".into());
    }
    match a {
        Array::Numeric(m) => Ok(m.data()[0]),
        Array::Typed(t) => Ok(t.real(0)),
        Array::Logical(l) => Ok(if l.data()[0] != 0 { 1.0 } else { 0.0 }),
        Array::Duration(d) if !d.is_calendar() => Ok(d.data()[0]),
        _ => Err(unsupported("retime", "non-numeric Constant")),
    }
}

fn timestep_grid(
    e: &Engine,
    first_t: i128,
    last_t: i128,
    step: i128,
    method: &str,
    who: &str,
    result: &mut Vec<i128>,
) -> Result<Option<i128>> {
    let first = first_t.div_euclid(step) * step;
    if (last_t - first) / step >= MAX_TIME_ROWS as i128 {
        return Err(format!("{who}: workspace exceeds 256 MiB"));
    }
    let mut n = first;
    while n <= last_t {
        if result.len() >= MAX_TIME_ROWS {
            return Err(format!("{who}: output exceeds 16777216 rows"));
        }
        if result.len().is_multiple_of(4096) {
            e.check_cancel()?;
        }
        result.push(n);
        n += step;
    }
    if !is_aggregating(method) && result.last().is_some_and(|v| *v < last_t) {
        if result.len() >= MAX_TIME_ROWS {
            return Err(format!("{who}: output exceeds 16777216 rows"));
        }
        result.push(n);
        n += step;
    }
    Ok(Some(n))
}

fn sample_rate_grid(
    e: &Engine,
    first_t: i128,
    last_t: i128,
    fs: f64,
    method: &str,
    who: &str,
    result: &mut Vec<i128>,
) -> Result<Option<i128>> {
    let dt_ns = 1e9 / fs;
    if dt_ns.fract() == 0.0 && dt_ns < 1e18 {
        return timestep_grid(e, first_t, last_t, dt_ns as i128, method, who, result);
    }
    let bits = fs.to_bits();
    let mut mant = ((bits & ((1 << 52) - 1)) | (1 << 52)) as i128;
    let mut exp = ((bits >> 52) as i32 & 0x7ff) - 1023 - 52;
    let tz = mant.trailing_zeros();
    mant >>= tz;
    exp += tz as i32;

    let rational = if (0..=60).contains(&exp) {
        mant.checked_shl(exp as u32).map(|num| (num, 1_i128))
    } else if exp < 0 && (-exp) <= 52 {
        Some((mant, 1_i128 << (-exp)))
    } else {
        None
    };

    if let Some((num, den)) = rational
        && let (Some(ns_den), Some(first_scaled), Some(last_scaled)) = (
            1_000_000_000_i128.checked_mul(den),
            first_t.checked_mul(num),
            last_t.checked_mul(num),
        )
    {
        let k0 = first_scaled.div_euclid(ns_den);
        let k_end = last_scaled.div_euclid(ns_den);
        if k_end - k0 >= MAX_TIME_ROWS as i128 {
            return Err(format!("{who}: workspace exceeds 256 MiB"));
        }
        let t_at = |k: i128| -> Option<i128> {
            k.checked_mul(2 * ns_den)
                .and_then(|v| v.checked_add(num))
                .map(|v| v.div_euclid(2 * num))
        };
        if t_at(k0).is_some() && t_at(k_end + 2).is_some() {
            let mut i = 0_i128;
            let mut n = t_at(k0).unwrap();
            while n <= last_t {
                if result.len() >= MAX_TIME_ROWS {
                    return Err(format!("{who}: output exceeds 16777216 rows"));
                }
                if result.len().is_multiple_of(4096) {
                    e.check_cancel()?;
                }
                result.push(n);
                i += 1;
                n = t_at(k0 + i).unwrap();
            }
            if !is_aggregating(method) && result.last().is_some_and(|v| *v < last_t) {
                if result.len() >= MAX_TIME_ROWS {
                    return Err(format!("{who}: output exceeds 16777216 rows"));
                }
                result.push(n);
                i += 1;
                n = t_at(k0 + i).unwrap();
            }
            return Ok(Some(n));
        }
    }

    let k0 = (first_t as f64 / dt_ns).floor();
    let est = ((last_t - (k0 * dt_ns).round() as i128) as f64 / dt_ns).floor();
    if !est.is_finite() || est >= MAX_TIME_ROWS as f64 {
        return Err(format!("{who}: workspace exceeds 256 MiB"));
    }
    let dt_int = dt_ns.floor() as i128;
    let dt_frac = dt_ns - dt_int as f64;
    let t_at_f64 = |k: f64| -> i128 { (k as i128) * dt_int + (k * dt_frac).round() as i128 };
    let mut i = 0usize;
    let mut n = t_at_f64(k0);
    while n <= last_t {
        if result.len() >= MAX_TIME_ROWS {
            return Err(format!("{who}: output exceeds 16777216 rows"));
        }
        if result.len().is_multiple_of(4096) {
            e.check_cancel()?;
        }
        result.push(n);
        i += 1;
        n = t_at_f64(k0 + i as f64);
    }
    if !is_aggregating(method) && result.last().is_some_and(|v| *v < last_t) {
        if result.len() >= MAX_TIME_ROWS {
            return Err(format!("{who}: output exceeds 16777216 rows"));
        }
        result.push(n);
        i += 1;
        n = t_at_f64(k0 + i as f64);
    }
    Ok(Some(n))
}

fn regular_grid(
    e: &mut Engine,
    template: &Array,
    first_t: i128,
    last_t: i128,
    spec: RegularSpec,
    method: &str,
    who: &str,
) -> Result<(Array, Option<i128>)> {
    if first_t > last_t {
        return Ok((time_array(template, &[])?, None));
    }
    let mut result = Vec::new();
    let mut next_edge = None;
    match spec {
        RegularSpec::TimeStep(step) => {
            next_edge = timestep_grid(e, first_t, last_t, step, method, who, &mut result)?;
        }
        RegularSpec::SampleRate(fs) => {
            next_edge = sample_rate_grid(e, first_t, last_t, fs, method, who, &mut result)?;
        }
        RegularSpec::CalendarStep(by) => {
            if !matches!(template, Array::Datetime(_)) {
                return Err(unsupported(who, "calendar TimeStep for duration row times"));
            }
            let first = first_t.div_euclid(NANOS_PER_DAY as i128) * NANOS_PER_DAY as i128;
            let base = time_array(template, &[first])?;
            let mut i = 0usize;
            loop {
                if i.is_multiple_of(4096) {
                    e.check_cancel()?;
                }
                let scale = Value::Matrix(Matrix::scalar(i as f64));
                let delta = e.dtm_binary(
                    &crate::syntax::Token::Star,
                    &Value::from(by.clone()),
                    &scale,
                )?;
                let value = e
                    .dtm_binary(
                        &crate::syntax::Token::Plus,
                        &Value::from(base.clone()),
                        &delta,
                    )?
                    .array(&e.cancel)?;
                let n = times(&value, e)?[0];
                if i == 1 && n <= first {
                    return Err(format!("{who}: TimeStep must advance time"));
                }
                if n > last_t {
                    if !is_aggregating(method) && result.last().is_some_and(|v| *v < last_t) {
                        if result.len() >= MAX_TIME_ROWS {
                            return Err(format!("{who}: output exceeds 16777216 rows"));
                        }
                        result.push(n);
                    } else {
                        next_edge = Some(n);
                    }
                    break;
                }
                if result.len() >= MAX_TIME_ROWS {
                    return Err(format!("{who}: output exceeds 16777216 rows"));
                }
                result.push(n);
                i += 1;
            }
        }
    }
    Ok((time_array(template, &result)?, next_edge))
}

fn target_times(
    e: &Engine,
    t: &TableArray,
    target: &Value,
    method: &str,
) -> Result<(Array, Option<i128>)> {
    if t.height() > MAX_TIME_ROWS {
        return Err("retime: workspace exceeds 256 MiB".into());
    }
    let template = t.row_times().unwrap();
    if let Some(unit) = text_scalar(target) {
        if ![
            "secondly", "minutely", "hourly", "daily", "monthly", "yearly",
        ]
        .contains(&unit.as_str())
        {
            return Err(unsupported("retime", &unit));
        }
        let x = times(template, e)?;
        sorted(&x, false)?;
        if x.is_empty() {
            return Ok((time_array(template, &[])?, None));
        }
        let step = match unit.as_str() {
            "secondly" => 1_000_000_000,
            "minutely" => 60_000_000_000,
            "hourly" => 3_600_000_000_000,
            _ => NANOS_PER_DAY as i128,
        };
        let mut first = x[0].div_euclid(step) * step;
        if unit == "monthly" || unit == "yearly" {
            if !matches!(template, Array::Datetime(_)) {
                return Err(unsupported("retime", "monthly/yearly duration row times"));
            }
            let (y, m, _) = super::super::dates::civil((first / NANOS_PER_DAY as i128) as i64);
            first = super::super::dates::civil_serial(y, if unit == "yearly" { 1 } else { m }, 1)
                as i128
                * NANOS_PER_DAY as i128;
        }
        if !["monthly", "yearly"].contains(&unit.as_str())
            && (x[x.len() - 1] - first) / step >= MAX_TIME_ROWS as i128
        {
            return Err("retime: workspace exceeds 256 MiB".into());
        }
        let mut result = Vec::new();
        let mut n = first;
        while n <= *x.last().unwrap() {
            if result.len() >= MAX_TIME_ROWS {
                return Err("retime: output exceeds 16777216 rows".into());
            }
            if result.len() % 4096 == 0 {
                e.check_cancel()?;
            }
            result.push(n);
            n = advance(n, &unit)?;
        }
        if ![
            "mean",
            "sum",
            "min",
            "max",
            "count",
            "firstvalue",
            "lastvalue",
        ]
        .contains(&method)
            && result.last().is_some_and(|v| v < x.last().unwrap())
        {
            if result.len() >= MAX_TIME_ROWS {
                return Err("retime: output exceeds 16777216 rows".into());
            }
            result.push(n);
            n = advance(n, &unit)?;
        }
        Ok((time_array(template, &result)?, Some(n)))
    } else {
        let a = crate::table_array::row_times_column(target.clone().array(&e.cancel)?)?;
        same_time_type(template, &a)?;
        if a.cols() != 1 {
            return Err("retime: newTimes must be a column vector".into());
        }
        let x = times(&a, e)?;
        sorted(&x, true)?;
        Ok((a, None))
    }
}

/// Gather row indices once per variable, preserving its class and temporal
/// metadata. Missing fixed-width values use that class's standard sentinel.
fn selected_column(
    e: &mut Engine,
    column: &Array,
    picks: &[Option<usize>],
    fill_constant: Option<f64>,
    out: &mut Execution,
) -> Result<Array> {
    let r = picks.len();
    let c = column.cols();
    if picks.iter().all(Option::is_some) {
        let rows: Vec<_> = picks.iter().map(|i| i.unwrap()).collect();
        return e.tbl_take_rows(column, &rows, out);
    }
    let positions: Vec<Option<usize>> =
        super::super::datetime_fns::polled(&e.cancel, r * c, |i| {
            picks[i % r].map(|j| (i / r) * column.rows() + j)
        })?;
    let gather = |missing: f64, value: &dyn Fn(usize) -> f64| {
        super::super::datetime_fns::polled(&e.cancel, r * c, |i| {
            positions[i].map(value).unwrap_or(missing)
        })
    };
    Ok(match column {
        Array::Numeric(m) => Matrix::from_column_vec(
            r,
            c,
            gather(fill_constant.unwrap_or(f64::NAN), &|i| m.data()[i])?,
        )?
        .into(),
        Array::Logical(m) => LogicalMatrix::from_column_vec(
            r,
            c,
            super::super::datetime_fns::polled(&e.cancel, r * c, |i| {
                positions[i]
                    .map(|j| m.data()[j])
                    .unwrap_or(fill_constant.map_or(0, |v| u8::from(v != 0.0)))
            })?,
            &e.cancel,
        )?
        .into(),
        Array::Typed(t) if t.is_integer() => crate::TypedArray::from_ints(
            t.class(),
            r,
            c,
            super::super::datetime_fns::polled(&e.cancel, r * c, |i| {
                positions[i]
                    .map(|j| t.int(j))
                    .unwrap_or(fill_constant.map_or(0, |v| v.round() as i128))
            })?
            .into_iter(),
        )?
        .into(),
        Array::Typed(t) => crate::TypedArray::from_reals(
            t.class(),
            r,
            c,
            gather(fill_constant.unwrap_or(f64::NAN), &|i| t.real(i))?.into_iter(),
        )?
        .into(),
        Array::Datetime(t) => {
            if fill_constant.is_some() {
                return Err(unsupported(
                    "retime",
                    "fillwithconstant for datetime variables",
                ));
            }
            t.with_data(
                r,
                c,
                super::super::datetime_fns::polled(&e.cancel, r * c, |i| {
                    positions[i]
                        .map(|j| t.data()[j])
                        .unwrap_or(DateTimeValue::NAT)
                })?,
            )?
            .into()
        }
        Array::Duration(d) if d.is_calendar() => {
            if fill_constant.is_some() {
                return Err(unsupported(
                    "retime",
                    "fillwithconstant for calendarDuration variables",
                ));
            }
            DurationArray::new_calendar(
                r,
                c,
                super::super::datetime_fns::polled(&e.cancel, r * c, |i| {
                    positions[i]
                        .map(|j| d.calendar_data().unwrap()[j])
                        .unwrap_or([f64::NAN; 4])
                })?,
            )?
            .into()
        }
        Array::Duration(d) => d
            .with_data(
                r,
                c,
                gather(fill_constant.unwrap_or(f64::NAN), &|i| d.data()[i])?,
            )?
            .into(),
        Array::Str(s) => {
            if fill_constant.is_some() {
                return Err(unsupported(
                    "retime",
                    "fillwithconstant for string variables",
                ));
            }
            Array::Str(crate::StringArray::new(
                r,
                c,
                super::super::datetime_fns::polled(&e.cancel, r * c, |i| {
                    positions[i].and_then(|j| s.data()[j].clone())
                })?,
            )?)
        }
        Array::Categorical(a) => {
            if fill_constant.is_some() {
                return Err(unsupported(
                    "retime",
                    "fillwithconstant for categorical variables",
                ));
            }
            crate::CategoricalArray::new(
                r,
                c,
                super::super::datetime_fns::polled(&e.cancel, r * c, |i| {
                    positions[i].map(|j| a.codes()[j]).unwrap_or(0)
                })?,
                a.categories().clone(),
                a.is_ordinal(),
            )?
            .into()
        }
        Array::Complex(m) => crate::ComplexMatrix::from_column_vec(
            r,
            c,
            super::super::datetime_fns::polled(&e.cancel, r * c, |i| {
                positions[i].map(|j| m.data()[j]).unwrap_or_else(|| {
                    fill_constant.map_or(crate::Complex64::new(f64::NAN, f64::NAN), |v| {
                        crate::Complex64::new(v, 0.0)
                    })
                })
            })?,
            &e.cancel,
        )?
        .into(),
        // Char/cellstr copying uses the existing class-aware index/write path.
        Array::Cell(_) | Array::Character(_) => {
            if fill_constant.is_some() {
                return Err(unsupported(
                    "retime",
                    "fillwithconstant for cell or char variables",
                ));
            }
            let mut rows = Vec::with_capacity(r);
            for (i, pick) in picks.iter().enumerate() {
                if i % 4096 == 0 {
                    e.check_cancel()?;
                }
                let row = if let Some(i) = pick {
                    e.tbl_take_rows(column, &[*i], out)?
                } else {
                    let piece = e.missing_piece(column, out)?.ok_or_else(|| {
                        unsupported("retime", "missing values for this variable type")
                    })?;
                    if c == 1 {
                        piece
                    } else {
                        e.tbl_join(vec![vec![Value::from(piece); c]], out)?
                            .array(&e.cancel)?
                    }
                };
                rows.push(vec![Value::from(row)]);
            }
            e.tbl_join(rows, out)?.array(&e.cancel)?
        }
        _ => {
            return Err(unsupported(
                "retime",
                "missing values for this variable type",
            ));
        }
    })
}

const METHODS: &[&str] = &[
    "fillwithmissing",
    "fillwithconstant",
    "previous",
    "next",
    "nearest",
    "linear",
    "spline",
    "mean",
    "sum",
    "min",
    "max",
    "count",
    "firstvalue",
    "lastvalue",
];
fn resample(
    e: &mut Engine,
    t: &TableArray,
    target: &Array,
    last_edge: Option<i128>,
    method: &str,
    out: &mut Execution,
) -> Result<TableArray> {
    resample_with_constant(e, t, target, last_edge, method, 0.0, out)
}

fn resample_with_constant(
    e: &mut Engine,
    t: &TableArray,
    target: &Array,
    last_edge: Option<i128>,
    method: &str,
    constant: f64,
    out: &mut Execution,
) -> Result<TableArray> {
    if !METHODS.contains(&method) {
        return Err(unsupported("retime", &format!("method '{method}'")));
    }
    let cells = t
        .columns()
        .iter()
        .try_fold(target.len(), |n, c| {
            n.checked_add(target.len().checked_mul(c.cols())?)
        })
        .ok_or("retime: workspace dimensions overflow")?;
    let bytes = t
        .height()
        .checked_mul(64)
        .and_then(|v| v.checked_add(target.len().checked_mul(48)?))
        .and_then(|v| v.checked_add(cells.checked_mul(40)?));
    if bytes.is_none_or(|v| v > 256 * 1024 * 1024) {
        return Err("retime: workspace exceeds 256 MiB".into());
    }
    let x = times(t.row_times().unwrap(), e)?;
    let q = times(target, e)?;
    let aggregate = [
        "mean",
        "sum",
        "min",
        "max",
        "count",
        "firstvalue",
        "lastvalue",
    ]
    .contains(&method);
    sorted(&x, ["linear", "spline"].contains(&method))?;
    sorted(&q, true)?;
    let mut p = t.to_parts();
    p.height = q.len();
    p.row_times = Some(target.clone());
    p.row_names = None;
    if q.is_empty() {
        for (i, column) in t.columns().iter().enumerate() {
            p.columns[i] = e.tbl_take_rows(column, &[], out)?;
        }
        return TableArray::new(p);
    }
    let total = t
        .columns()
        .iter()
        .try_fold(q.len(), |n, c| {
            n.checked_add(q.len().checked_mul(c.cols())?)
        })
        .ok_or("retime: output dimensions overflow")?;
    if total > MAX_ELEMENTS {
        return Err("retime: output exceeds 16777216 elements".into());
    }
    // Compute source indices once, then share across every variable and component.
    let mut picks = Vec::with_capacity(q.len());
    for (j, &n) in q.iter().enumerate() {
        if j % 4096 == 0 {
            e.check_cancel()?;
        }
        let hi = x.partition_point(|&v| v < n);
        let exact = hi < x.len() && x[hi] == n;
        let pick = if exact {
            Some(if method == "previous" {
                x.partition_point(|&v| v <= n) - 1
            } else {
                hi
            })
        } else {
            match method {
                // No neighbor before the first row, or after the last.
                "previous" if hi > 0 => Some(hi - 1),
                "next" if hi < x.len() => Some(hi),
                "nearest" if !x.is_empty() => Some(if hi == 0 {
                    0
                } else if hi == x.len() || n - x[hi - 1] < x[hi] - n {
                    hi - 1
                } else {
                    hi
                }),
                _ => None,
            }
        };
        picks.push(pick);
    }
    let fill_constant = (method == "fillwithconstant").then_some(constant);
    for (v, column) in t.columns().iter().enumerate() {
        e.check_cancel()?;
        if !aggregate && !["linear", "spline"].contains(&method) {
            p.columns[v] = selected_column(e, column, &picks, fill_constant, out)?;
            continue;
        }
        let m = Value::from(column.clone())
            .matrix(&e.cancel)
            .map_err(|_| unsupported("retime", "this variable type for this method"))?;
        let mut values = vec![f64::NAN; q.len() * m.cols()];
        for c in 0..m.cols() {
            let lane = &m.data()[c * x.len()..(c + 1) * x.len()];
            if method == "spline" && x.len() >= 2 {
                let origin = x[0];
                let num = |r, cs, d| -> Result<Value> {
                    Ok(Value::Matrix(Matrix::from_column_vec(r, cs, d)?))
                };
                let xi = num(
                    x.len(),
                    1,
                    x.iter().map(|n| (n - origin) as f64 / 1e9).collect(),
                )?;
                let yi = num(x.len(), 1, lane.to_vec())?;
                let qi = num(
                    q.len(),
                    1,
                    q.iter().map(|n| (n - origin) as f64 / 1e9).collect(),
                )?;
                let result = e
                    .polyopt_interp1(&[xi, yi, qi, Value::Text("spline".into())])?
                    .matrix(&e.cancel)?;
                values[c * q.len()..(c + 1) * q.len()].copy_from_slice(result.data());
                continue;
            }
            for (j, &n) in q.iter().enumerate() {
                if j % 4096 == 0 {
                    e.check_cancel()?;
                }
                let val = if aggregate {
                    let lo = x.partition_point(|&v| v < n);
                    let end = q.get(j + 1).copied().or(last_edge).or_else(|| {
                        if q.len() > 1 {
                            Some(n + q[j] - q[j - 1])
                        } else {
                            None
                        }
                    });
                    let hi = end
                        // With left-included bins the final bin includes both
                        // edges; interior bins remain half open.
                        .map(|b| x.partition_point(|&v| v < b || (j + 1 == q.len() && v == b)))
                        .unwrap_or(x.len());
                    let mut count = 0;
                    let mut sum = 0.;
                    let mut min = f64::INFINITY;
                    let mut max = f64::NEG_INFINITY;
                    let mut first = f64::NAN;
                    let mut last = f64::NAN;
                    for (k, &a) in lane[lo..hi].iter().enumerate() {
                        if k % 4096 == 0 {
                            e.check_cancel()?;
                        }
                        if a.is_nan() {
                            continue;
                        }
                        if count == 0 {
                            first = a;
                        }
                        last = a;
                        count += 1;
                        sum += a;
                        min = min.min(a);
                        max = max.max(a);
                    }
                    match method {
                        "sum" => sum,
                        "count" => count as f64,
                        _ if count == 0 => f64::NAN,
                        "mean" => sum / count as f64,
                        "min" => min,
                        "max" => max,
                        "firstvalue" => first,
                        "lastvalue" => last,
                        _ => unreachable!(),
                    }
                } else if let Some(i) = picks[j] {
                    lane[i]
                } else if method == "linear" {
                    let hi = x.partition_point(|&v| v < n);
                    if x.len() < 2 {
                        f64::NAN
                    } else {
                        let hi = hi.clamp(1, x.len() - 1);
                        let a = (n - x[hi - 1]) as f64 / (x[hi] - x[hi - 1]) as f64;
                        lane[hi - 1] * (1. - a) + lane[hi] * a
                    }
                } else {
                    f64::NAN
                };
                values[c * q.len() + j] = val;
            }
        }
        p.columns[v] = Matrix::from_column_vec(q.len(), m.cols(), values)?.into();
    }
    TableArray::new(p)
}

fn synchronize(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    if !(2..=6).contains(&args.len()) {
        return Err(unsupported("synchronize", "this syntax"));
    }
    let a = timetable(args, "synchronize")?;
    let b = timetable(&args[1..], "synchronize")?;
    if a.height()
        .checked_add(b.height())
        .and_then(|n| n.checked_mul(128))
        .is_none_or(|n| n > 256 * 1024 * 1024)
    {
        return Err("synchronize: workspace exceeds 256 MiB".into());
    }
    let at = a.row_times().unwrap();
    let bt = b.row_times().unwrap();
    same_time_type(at, bt)?;
    let x = times(at, e)?;
    let y = times(bt, e)?;
    sorted(&x, true)?;
    sorted(&y, true)?;
    let basis = args
        .get(2)
        .map(|v| text_scalar(v).ok_or("synchronize: time basis must be text"))
        .transpose()?
        .unwrap_or("union".into());
    let (method, target, last_edge) = if basis == "regular" {
        let (method, opt_idx) = match args.len() {
            5 => ("fillwithmissing".to_string(), 3),
            6 => (
                text_scalar(&args[3]).ok_or("synchronize: method must be text")?,
                4,
            ),
            _ => {
                return Err("synchronize: 'regular' requires 'TimeStep' or 'SampleRate'".into());
            }
        };
        if !METHODS.contains(&method.as_str()) {
            return Err(unsupported("synchronize", &format!("method '{method}'")));
        }
        let spec = parse_regular_spec(e, &args[opt_idx], &args[opt_idx + 1], "synchronize")?;
        let first_t = match (x.first(), y.first()) {
            (Some(&fa), Some(&fb)) => Some(fa.min(fb)),
            (Some(&f), None) | (None, Some(&f)) => Some(f),
            (None, None) => None,
        };
        let last_t = match (x.last(), y.last()) {
            (Some(&la), Some(&lb)) => Some(la.max(lb)),
            (Some(&l), None) | (None, Some(&l)) => Some(l),
            (None, None) => None,
        };
        let (target, last_edge) = match (first_t, last_t) {
            (Some(ft), Some(lt)) => regular_grid(e, at, ft, lt, spec, &method, "synchronize")?,
            _ => (time_array(at, &[])?, None),
        };
        (method, target, last_edge)
    } else {
        if args.len() > 4 {
            return Err(unsupported("synchronize", "this syntax"));
        }
        let method = args
            .get(3)
            .map(|v| text_scalar(v).ok_or("synchronize: method must be text"))
            .transpose()?
            .unwrap_or("fillwithmissing".into());
        let mut q = match basis.as_str() {
            "first" => x.clone(),
            "union" | "commonrange" => {
                let mut v = x.clone();
                v.extend_from_slice(&y);
                v.sort_unstable();
                v.dedup();
                v
            }
            "intersection" => x
                .iter()
                .copied()
                .filter(|n| y.binary_search(n).is_ok())
                .collect(),
            _ => return Err(unsupported("synchronize", &basis)),
        };
        if basis == "commonrange" {
            if x.is_empty() || y.is_empty() {
                q.clear();
            } else {
                q.retain(|n| *n >= x[0].max(y[0]) && *n <= x[x.len() - 1].min(y[y.len() - 1]));
            }
        }
        if q.len() > MAX_ELEMENTS {
            return Err("synchronize: output exceeds 16777216 rows".into());
        }
        (method, time_array(at, &q)?, None)
    };
    let aa = resample(e, a, &target, last_edge, &method, out)?;
    let bb = resample(e, b, &target, last_edge, &method, out)?;
    let mut p = aa.to_parts();
    let bp = bb.parts();
    if p.units.is_empty() && !bp.units.is_empty() {
        p.units = vec![String::new(); p.names.len()];
    }
    if p.descriptions.is_empty() && !bp.descriptions.is_empty() {
        p.descriptions = vec![String::new(); p.names.len()];
    }
    let duplicate: std::collections::HashSet<_> = a
        .names()
        .iter()
        .filter(|n| b.names().contains(n))
        .cloned()
        .collect();
    for n in &mut p.names {
        if duplicate.contains(n) {
            *n = format!("{}_TT1", n.chars().take(59).collect::<String>());
        }
    }
    for (i, n) in bp.names.iter().enumerate() {
        let n = if duplicate.contains(n) {
            format!("{}_TT2", n.chars().take(59).collect::<String>())
        } else {
            n.clone()
        };
        p.push_variable(n, bp.columns[i].clone());
        let last = p.names.len() - 1;
        if !bp.units.is_empty() {
            p.units[last] = bp.units[i].clone();
        }
        if !bp.descriptions.is_empty() {
            p.descriptions[last] = bp.descriptions[i].clone();
        }
    }
    Ok(tvalue(TableArray::new(p)?))
}
