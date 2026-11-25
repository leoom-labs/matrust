//! Calendar units remain separate from elapsed milliseconds. There is no
//! conversion of months to a fixed number of days. Month arithmetic clamps
//! to the destination month's last day, then adds days and elapsed time.
use super::*;
use crate::datetime_array::NANOS_PER_DAY;
use crate::syntax::Token;

pub(super) const NAMES: &[&str] = &[
    "calendarDuration",
    "calyears",
    "calmonths",
    "calweeks",
    "caldays",
    "iscalendarduration",
    "between",
];

pub(super) fn of(v: &Value) -> Option<&DurationArray> {
    duration_of(v).filter(|d| d.is_calendar())
}

fn fixed_duration(v: &Value) -> Option<&DurationArray> {
    duration_of(v).filter(|d| !d.is_calendar())
}
pub(super) fn is_calendar(v: &Value) -> bool {
    of(v).is_some()
}

fn unsupported(name: &str) -> String {
    format!("calendarDuration: {name} is not supported")
}

pub(super) fn render(v: [f64; 4]) -> String {
    if v.iter().any(|n| n.is_nan()) {
        return "NaN".into();
    }
    let [y, m, d, ms] = v;
    let mut words = Vec::new();
    if y != 0. {
        words.push(format!("{y}y"));
    }
    if m != 0. {
        words.push(format!("{m}mo"));
    }
    if d != 0. {
        words.push(format!("{d}d"));
    }
    if ms != 0. {
        let h = (ms / 3_600_000.).trunc();
        let mi = ((ms - h * 3_600_000.) / 60_000.).trunc();
        let s = (ms - h * 3_600_000. - mi * 60_000.) / 1000.;
        words.push(format!("{h}h {mi}m {s}s"));
    }
    if words.is_empty() {
        "0d".into()
    } else {
        words.join(" ")
    }
}

pub(super) fn normalized(mut v: [f64; 4]) -> Result<[f64; 4]> {
    if v.iter().any(|x| x.is_nan()) {
        return Ok([f64::NAN; 4]);
    }
    if v.iter().any(|x| !x.is_finite() || x.abs() >= 1e15) {
        return Err(
            "calendarDuration: components must be finite and below 1e15 in magnitude".into(),
        );
    }
    if v[..3].iter().any(|x| x.fract() != 0.) {
        return Err("calendarDuration: years, months and days must be integers".into());
    }
    let months = v[0] * 12. + v[1];
    v[0] = (months / 12.).trunc();
    v[1] = months % 12.;
    Ok(v)
}

pub(super) fn outputs(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
) -> Result<Option<Vec<Value>>> {
    if !NAMES.contains(&name) && !args.iter().any(is_calendar) {
        return Ok(None);
    }
    if NAMES.contains(&name) && count > 1 {
        return Err(format!("{name}: too many outputs"));
    }
    let one = |v| Ok(Some(vec![v]));
    match name {
        "calendarDuration" => {
            if ![3, 6].contains(&args.len()) {
                return Err("calendarDuration: this syntax or options is not supported".into());
            }
            let mut numbers = Vec::new();
            let mut shape = (1, 1);
            for a in args {
                let (v, s) = e.dtm_numbers(a, name)?;
                shape = broadcast(shape, s, name)?;
                numbers.push((v, s));
            }
            crate::duration_array::checked_calendar_len(shape.0, shape.1)?;
            let mut data = Vec::with_capacity(shape.0 * shape.1);
            for i in 0..shape.0 * shape.1 {
                e.dtm_check(i)?;
                let mut c = [0.; 6];
                for (j, (v, s)) in numbers.iter().enumerate() {
                    c[j] = v[expand_index(i, shape.0, *s)];
                }
                data.push(normalized([
                    c[0],
                    c[1],
                    c[2],
                    c[3] * 3_600_000. + c[4] * 60_000. + c[5] * 1000.,
                ])?);
            }
            one(du_value(DurationArray::new_calendar(
                shape.0, shape.1, data,
            )?))
        }
        "calyears" | "calmonths" | "caldays" | "calweeks" => {
            let [a] = args else {
                return Err(format!("{name}: expected one input"));
            };
            if let Some(d) = of(a) {
                let values = polled_map(&e.cancel, d.calendar_data().unwrap(), |v| match name {
                    "calyears" => v[0],
                    "calmonths" => 12. * v[0] + v[1],
                    "calweeks" => (v[2] / 7.).trunc(),
                    _ => v[2],
                })?;
                return one(number_matrix(d.rows(), d.cols(), values)?);
            }
            let (values, shape) = e.dtm_numbers(a, name)?;
            crate::duration_array::checked_calendar_len(shape.0, shape.1)?;
            let mut data = Vec::with_capacity(values.len());
            for (i, x) in values.into_iter().enumerate() {
                e.dtm_check(i)?;
                let v = match name {
                    "calyears" => [x, 0., 0., 0.],
                    "calmonths" => [0., x, 0., 0.],
                    "calweeks" => [0., 0., x * 7., 0.],
                    _ => [0., 0., x, 0.],
                };
                if x.fract() != 0. && !x.is_nan() {
                    return Err(format!("{name}: input must contain integers"));
                }
                data.push(normalized(v)?);
            }
            one(du_value(DurationArray::new_calendar(
                shape.0, shape.1, data,
            )?))
        }
        "iscalendarduration" | "isduration" | "isdatetime" => {
            if args.len() != 1 {
                return Err(format!("{name}: expected one input"));
            }
            one(scalar_bool(
                name == "iscalendarduration" && is_calendar(&args[0]),
            ))
        }
        "split" => split(e, args, count),
        "between" => one(between(e, args)?),
        "isequal" | "isequaln" => {
            if args.len() < 2 {
                return Err(format!("{name}: expected at least two inputs"));
            }
            let a = of(&args[0]);
            let mut equal = a.is_some();
            if let Some(a) = a {
                for b in &args[1..] {
                    let Some(b) = of(b) else {
                        equal = false;
                        break;
                    };
                    equal &= a.rows() == b.rows() && a.cols() == b.cols();
                    if !equal {
                        break;
                    }
                    for (i, (x, y)) in a
                        .calendar_data()
                        .unwrap()
                        .iter()
                        .zip(b.calendar_data().unwrap())
                        .enumerate()
                    {
                        e.dtm_check(i)?;
                        equal &= x.iter().zip(y).all(|(x, y)| {
                            x == y || (name == "isequaln" && x.is_nan() && y.is_nan())
                        });
                        if !equal {
                            break;
                        }
                    }
                }
            }
            one(scalar_bool(equal))
        }
        // These existing collection paths preserve shared storage or only
        // inspect shape/class or use the calendar-aware text renderer.
        "class" | "isa" | "size" | "numel" | "length" | "ndims" | "isempty" | "isnumeric"
        | "ischar" | "iscell" | "isstruct" | "islogical" | "isfloat" | "isinteger" | "isstring"
        | "issparse" | "isreal" | "isvector" | "isrow" | "iscolumn" | "isscalar" | "ismatrix"
        | "reshape" | "string" | "char" | "cellstr" => Ok(None),
        _ if !args.iter().any(is_calendar) => Ok(None),
        _ => Err(unsupported(name)),
    }
}

fn split(e: &Engine, args: &[Value], count: usize) -> Result<Option<Vec<Value>>> {
    let Some(d) = args.first().and_then(of) else {
        return Ok(None);
    };
    if args.len() > 2 {
        return Err(unsupported("split options"));
    }
    let units = if let Some(v) = args.get(1) {
        text_elements(v, &e.cancel)?
            .ok_or("split: units must be text")?
            .0
            .into_iter()
            .map(|v| v.ok_or("split: missing units are not supported".into()))
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![
            "years".into(),
            "months".into(),
            "days".into(),
            "time".into(),
        ]
    };
    let order = ["years", "months", "days", "time"];
    let mut last = None;
    for unit in &units {
        let k = order
            .iter()
            .position(|u| u == unit)
            .ok_or_else(|| unsupported(&format!("split unit '{unit}'")))?;
        if last.is_some_and(|p| p >= k) {
            return Err("split: units must be unique and ordered years, months, days, time".into());
        }
        last = Some(k);
    }
    if count > units.len() || units.is_empty() {
        return Err("split: too many outputs or no units".into());
    }
    let mut values = Vec::new();
    for unit in units.iter().take(count) {
        let numbers = polled_map(&e.cancel, d.calendar_data().unwrap(), |v| {
            match unit.as_str() {
                "years" => v[0],
                "months" => {
                    v[1] + if units.iter().any(|u| u == "years") {
                        0.
                    } else {
                        12. * v[0]
                    }
                }
                "days" => v[2],
                _ => v[3],
            }
        })?;
        values.push(if unit == "time" {
            du_value(DurationArray::new(d.rows(), d.cols(), numbers, "hh:mm:ss")?)
        } else {
            number_matrix(d.rows(), d.cols(), numbers)?
        });
    }
    Ok(Some(values))
}

/// Larger units are applied first, with month-end clamping at each step.
fn shift(t: DateTimeValue, c: [f64; 4]) -> Result<DateTimeValue> {
    if t.is_nat() || c.iter().any(|v| v.is_nan()) {
        return Ok(DateTimeValue::NAT);
    }
    let c = normalized(c)?;
    let p = calendar::parts(t);
    let y = p.y as i128 + c[0] as i128;
    let d = p.d.min(calendar::month_length(
        y.clamp(-100_000_000, 100_000_000) as i64,
        p.mo,
    ));
    let first = calendar::from_fields(y, p.mo as i128, d as i128, t.nanos as i128)?;
    let p = calendar::parts(first);
    let m = p.mo as i128 + c[1] as i128;
    let yy = p.y as i128 + (m - 1).div_euclid(12);
    let mm = (m - 1).rem_euclid(12) as usize + 1;
    let dd = p.d.min(calendar::month_length(
        yy.clamp(-100_000_000, 100_000_000) as i64,
        mm,
    ));
    let mut result = calendar::from_fields(
        yy,
        mm as i128,
        dd as i128 + c[2] as i128,
        first.nanos as i128,
    )?;
    result = calendar::shifted(result, calendar::ms_to_nanos(c[3])?)?;
    Ok(result)
}

pub(super) fn binary(e: &Engine, op: &Token, a: &Value, b: &Value) -> Result<Value> {
    let ca = of(a);
    let cb = of(b);
    let ta = datetime_of(a);
    let tb = datetime_of(b);
    let shape_a = a.dims();
    let shape_b = b.dims();
    let sa = (shape_a[0], shape_a[1]);
    let sb = (shape_b[0], shape_b[1]);
    let shape = broadcast(sa, sb, "calendarDuration")?;
    let negate = matches!(op, Token::Minus);
    if (ta.is_some() && cb.is_some() && matches!(op, Token::Plus | Token::Minus))
        || (ca.is_some() && tb.is_some() && matches!(op, Token::Plus))
    {
        let (t, c, ts, cs) = if let (Some(t), Some(c)) = (ta, cb) {
            (t, c, sa, sb)
        } else {
            (tb.unwrap(), ca.unwrap(), sb, sa)
        };
        let mut data = Vec::with_capacity(shape.0 * shape.1);
        for i in 0..shape.0 * shape.1 {
            e.dtm_check(i)?;
            let mut c = c.calendar_data().unwrap()[expand_index(i, shape.0, cs)];
            if negate {
                c = c.map(|n| -n);
            }
            data.push(shift(t.data()[expand_index(i, shape.0, ts)], c)?);
        }
        return Ok(dt_value(t.with_data(shape.0, shape.1, data)?));
    }
    crate::duration_array::checked_calendar_len(shape.0, shape.1)?;
    let mut data = Vec::with_capacity(shape.0 * shape.1);
    match (op, ca, cb) {
        (Token::Plus | Token::Minus, Some(a), Some(b)) => {
            for i in 0..shape.0 * shape.1 {
                e.dtm_check(i)?;
                let x = a.calendar_data().unwrap()[expand_index(i, shape.0, sa)];
                let y = b.calendar_data().unwrap()[expand_index(i, shape.0, sb)];
                data.push(normalized(std::array::from_fn(|j| {
                    if negate { x[j] - y[j] } else { x[j] + y[j] }
                }))?);
            }
        }
        (Token::Plus | Token::Minus, _, _) => {
            // A fixed duration adds into the elapsed-time component. Hours
            // stay hours: they are not folded into calendar days.
            let (cal, fixed, cs, fs, cal_left) =
                if let (Some(cal), Some(fixed)) = (ca, fixed_duration(b)) {
                    (cal, fixed, sa, sb, true)
                } else if let (Some(fixed), Some(cal)) = (fixed_duration(a), cb) {
                    (cal, fixed, sb, sa, false)
                } else {
                    return Err(unsupported("this arithmetic operation"));
                };
            for i in 0..shape.0 * shape.1 {
                e.dtm_check(i)?;
                let mut x = cal.calendar_data().unwrap()[expand_index(i, shape.0, cs)];
                let ms = fixed.data()[expand_index(i, shape.0, fs)];
                if negate && !cal_left {
                    x = x.map(|n| -n);
                    x[3] += ms;
                } else if negate {
                    x[3] -= ms;
                } else {
                    x[3] += ms;
                }
                data.push(normalized(x)?);
            }
        }
        (Token::Star | Token::DotStar, _, _) if ca.is_some() != cb.is_some() => {
            let (c, n, cs) = if let Some(c) = ca {
                (c, b, sa)
            } else {
                (cb.unwrap(), a, sb)
            };
            let (n, ns) = e.dtm_numbers(n, "calendarDuration")?;
            if n.len() != 1 {
                return Err(unsupported("multiplication by a nonscalar"));
            }
            for i in 0..shape.0 * shape.1 {
                e.dtm_check(i)?;
                let x = c.calendar_data().unwrap()[expand_index(i, shape.0, cs)];
                data.push(normalized(x.map(|v| v * n[expand_index(i, shape.0, ns)]))?);
            }
        }
        _ => return Err(unsupported("this arithmetic operation")),
    }
    Ok(du_value(DurationArray::new_calendar(
        shape.0, shape.1, data,
    )?))
}

pub(super) fn unary(e: &Engine, negative: bool, v: Value) -> Result<Value> {
    if !negative {
        return Ok(v);
    }
    let d = of(&v).unwrap();
    let data = polled_map(&e.cancel, d.calendar_data().unwrap(), |v| v.map(|x| -x))?;
    Ok(du_value(DurationArray::new_calendar(
        d.rows(),
        d.cols(),
        data,
    )?))
}

pub(super) fn concat(e: &Engine, rows: Vec<Vec<Value>>) -> Result<Value> {
    let mut grid = Vec::new();
    for row in rows {
        let mut blocks = Vec::new();
        for v in row {
            if v.dims().iter().product::<usize>() == 0 {
                continue;
            }
            let d = of(&v).ok_or_else(|| unsupported("concatenation with a different class"))?;
            blocks.push((d.rows(), d.cols(), d.calendar_data().unwrap().to_vec()));
        }
        grid.push(blocks);
    }
    let n = grid
        .iter()
        .flatten()
        .try_fold(0usize, |n, (_, _, v)| n.checked_add(v.len()))
        .ok_or("calendarDuration: dimensions overflow")?;
    crate::duration_array::checked_calendar_len(n, 1)?;
    let (r, c, data) = join_grid(grid, &e.cancel)?;
    Ok(du_value(DurationArray::new_calendar(r, c, data)?))
}

fn between(e: &Engine, args: &[Value]) -> Result<Value> {
    let [a, b] = args else {
        return Err("between: options are not supported; expected two datetimes".into());
    };
    let a = datetime_of(a).ok_or("between: expected datetime inputs")?;
    let b = datetime_of(b).ok_or("between: expected datetime inputs")?;
    if a.zone() != b.zone() {
        return Err("between: time zones must match".into());
    }
    let sa = (a.rows(), a.cols());
    let sb = (b.rows(), b.cols());
    let shape = broadcast(sa, sb, "between")?;
    crate::duration_array::checked_calendar_len(shape.0, shape.1)?;
    let mut data = Vec::with_capacity(shape.0 * shape.1);
    for i in 0..shape.0 * shape.1 {
        e.dtm_check(i)?;
        let a = a.data()[expand_index(i, shape.0, sa)];
        let b = b.data()[expand_index(i, shape.0, sb)];
        if a.is_nat() || b.is_nat() {
            data.push([f64::NAN; 4]);
            continue;
        }
        let pa = calendar::parts(a);
        let pb = calendar::parts(b);
        let mut m = (pb.y - pa.y) * 12 + pb.mo as i64 - pa.mo as i64;
        let mut c = normalized([0., m as f64, 0., 0.])?;
        let mut moved = shift(a, c)?;
        let sign = if b.total_nanos() >= a.total_nanos() {
            1
        } else {
            -1
        };
        if (sign == 1 && moved.total_nanos() > b.total_nanos())
            || (sign == -1 && moved.total_nanos() < b.total_nanos())
        {
            m -= sign;
            c = normalized([0., m as f64, 0., 0.])?;
            moved = shift(a, c)?;
        }
        let delta = b.total_nanos().unwrap() - moved.total_nanos().unwrap();
        c[2] = (delta / NANOS_PER_DAY as i128) as f64;
        c[3] = (delta % NANOS_PER_DAY as i128) as f64 / 1e6;
        data.push(c);
    }
    Ok(du_value(DurationArray::new_calendar(
        shape.0, shape.1, data,
    )?))
}
