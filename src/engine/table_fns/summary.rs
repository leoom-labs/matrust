//! `summary(T)` and `s = summary(T)` for tables and timetables.

use super::super::{Engine, Execution, Value};
use crate::datetime_array::{DateTimeValue, NANOS_PER_DAY};
use crate::table_array::TableArray;
use crate::{Array, CellArray, CharacterRow, DurationArray, Matrix, Result, StructArray};
use std::fmt::Write;
use std::sync::Arc;

pub(super) fn call(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Option<Vec<Value>>> {
    if args.len() != 1 {
        return Err("summary: expected one input".into());
    }
    let Some(t) = super::table_of(&args[0]) else {
        return Err("summary: expected a table or timetable".into());
    };
    if count == 0 {
        let text = format_table(e, t)?;
        e.emit(out, text)?;
        return Ok(Some(Vec::new()));
    }
    let s = build_struct(e, t)?;
    Ok(Some(vec![Value::Container(Array::Struct(s))]))
}

fn format_size(dims: &[usize]) -> String {
    dims.iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join("x")
}

fn class_name(a: &Array) -> &'static str {
    match a {
        Array::Numeric(_) => "double",
        Array::Logical(_) => "logical",
        Array::Complex(_) => "double",
        Array::Character(_) => "char",
        Array::Str(_) => "string",
        Array::Cell(_) => "cell",
        Array::Graphics(_) => "graphics",
        Array::Struct(_) => "struct",
        Array::Handle(_) => "function_handle",
        Array::Typed(t) => t.class().name(),
        Array::Categorical(_) => "categorical",
        Array::Datetime(_) => "datetime",
        Array::Duration(d) => {
            if d.is_calendar() {
                "calendarDuration"
            } else {
                "duration"
            }
        }
        Array::Table(t) => {
            if t.is_timetable() {
                "timetable"
            } else {
                "table"
            }
        }
        Array::Sparse(_) => "double",
    }
}

struct NumericStats {
    mins: Vec<f64>,
    meds: Vec<f64>,
    maxs: Vec<f64>,
    means: Vec<f64>,
    stds: Vec<f64>,
    miss: Vec<f64>,
}

fn safe_mid(a: f64, b: f64) -> f64 {
    if a == b {
        a
    } else if !a.is_finite() || !b.is_finite() || (a < 0.0) != (b < 0.0) {
        0.5 * (a + b)
    } else {
        a + 0.5 * (b - a)
    }
}

fn mean_and_std(valid: &[f64]) -> (f64, f64) {
    let n = valid.len();
    let max_abs = valid.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
    if !max_abs.is_finite() {
        let sum: f64 = valid.iter().sum();
        return (sum / n as f64, f64::NAN);
    }
    if max_abs == 0.0 {
        return (0.0, 0.0);
    }
    let mean_s: f64 = valid.iter().map(|&v| v / max_abs).sum::<f64>() / n as f64;
    let mean = mean_s * max_abs;
    let std = if n <= 1 {
        0.0
    } else {
        let ss: f64 = valid.iter().map(|&v| (v / max_abs - mean_s).powi(2)).sum();
        (ss / (n - 1) as f64).sqrt() * max_abs
    };
    (mean, std)
}

fn numeric_stats(rows: usize, cols: usize, get: impl Fn(usize) -> f64) -> NumericStats {
    let mut mins = Vec::with_capacity(cols);
    let mut meds = Vec::with_capacity(cols);
    let mut maxs = Vec::with_capacity(cols);
    let mut means = Vec::with_capacity(cols);
    let mut stds = Vec::with_capacity(cols);
    let mut miss = Vec::with_capacity(cols);
    for c in 0..cols {
        let mut valid = Vec::with_capacity(rows);
        let mut n_missing = 0usize;
        for r in 0..rows {
            let v = get(c * rows + r);
            if v.is_nan() {
                n_missing += 1;
            } else {
                valid.push(v);
            }
        }
        valid.sort_by(f64::total_cmp);
        if valid.is_empty() {
            mins.push(f64::NAN);
            meds.push(f64::NAN);
            maxs.push(f64::NAN);
            means.push(f64::NAN);
            stds.push(f64::NAN);
        } else {
            mins.push(valid[0]);
            maxs.push(valid[valid.len() - 1]);
            let mid = valid.len() / 2;
            let med = if valid.len() % 2 == 1 {
                valid[mid]
            } else {
                safe_mid(valid[mid - 1], valid[mid])
            };
            meds.push(med);
            let (mean, std) = mean_and_std(&valid);
            means.push(mean);
            stds.push(std);
        }
        miss.push(n_missing as f64);
    }
    NumericStats {
        mins,
        meds,
        maxs,
        means,
        stds,
        miss,
    }
}

fn dt_from_nanos(total: i128) -> DateTimeValue {
    let day = i128::from(NANOS_PER_DAY);
    DateTimeValue {
        days: total.div_euclid(day) as i64,
        nanos: total.rem_euclid(day) as i64,
    }
}

fn time_stats(data: &[DateTimeValue]) -> (DateTimeValue, DateTimeValue, DateTimeValue, usize) {
    let mut valid = Vec::with_capacity(data.len());
    let mut missing = 0usize;
    for &v in data {
        if let Some(ns) = v.total_nanos() {
            valid.push(ns);
        } else {
            missing += 1;
        }
    }
    valid.sort_unstable();
    if valid.is_empty() {
        return (
            DateTimeValue::NAT,
            DateTimeValue::NAT,
            DateTimeValue::NAT,
            missing,
        );
    }
    let min_v = dt_from_nanos(valid[0]);
    let max_v = dt_from_nanos(valid[valid.len() - 1]);
    let mid = valid.len() / 2;
    let med_ns = if valid.len() % 2 == 1 {
        valid[mid]
    } else {
        valid[mid - 1] + (valid[mid] - valid[mid - 1]) / 2
    };
    (min_v, dt_from_nanos(med_ns), max_v, missing)
}

fn format_scalar_time(e: &Engine, a: &Array) -> Result<String> {
    Ok(crate::display_format::disp(a, &e.display)?
        .unwrap_or_default()
        .trim()
        .to_string())
}

fn append_var_summary(
    e: &Engine,
    out: &mut String,
    name: &str,
    col: &Array,
    desc: &str,
    units: &str,
) -> Result<()> {
    let dims = col.dims();
    let sz = format_size(&dims);
    let cls = class_name(col);
    writeln!(out, "    {name}: {sz} {cls}").unwrap();
    if !desc.is_empty() {
        writeln!(out, "        Description:  {desc}").unwrap();
    }
    if !units.is_empty() {
        writeln!(out, "        Units:        {units}").unwrap();
    }
    let st = e.display.style();
    match col {
        Array::Numeric(m) if !m.is_nd() => {
            let stats = numeric_stats(m.rows(), m.cols(), |i| m.data()[i]);
            write_numeric_stats(out, &stats, &st);
        }
        Array::Typed(t) if !t.is_nd() => {
            let stats = numeric_stats(t.rows(), t.cols(), |i| t.real(i));
            write_numeric_stats(out, &stats, &st);
        }
        Array::Logical(l) => {
            let t_cnt = l.data().iter().filter(|&&b| b != 0).count();
            let f_cnt = l.len() - t_cnt;
            writeln!(out, "        Values:").unwrap();
            writeln!(out, "            True     {t_cnt}").unwrap();
            writeln!(out, "            False    {f_cnt}").unwrap();
        }
        Array::Categorical(c) => {
            let cats = c.categories();
            let mut counts = vec![0usize; cats.len()];
            let mut undef = 0usize;
            for &code in c.codes() {
                if code == 0 {
                    undef += 1;
                } else {
                    counts[code as usize - 1] += 1;
                }
            }
            writeln!(out, "        Values:").unwrap();
            for (cat_name, cnt) in cats.iter().zip(counts.iter()) {
                writeln!(out, "            {cat_name:<12} {cnt}").unwrap();
            }
            if undef > 0 {
                writeln!(out, "            {:<12} {undef}", "<undefined>").unwrap();
            }
        }
        Array::Datetime(dt) => {
            let (min_v, med_v, max_v, missing) = time_stats(dt.data());
            let s_min = format_scalar_time(e, &Array::Datetime(dt.with_data(1, 1, vec![min_v])?))?;
            let s_med = format_scalar_time(e, &Array::Datetime(dt.with_data(1, 1, vec![med_v])?))?;
            let s_max = format_scalar_time(e, &Array::Datetime(dt.with_data(1, 1, vec![max_v])?))?;
            writeln!(out, "        Values:").unwrap();
            writeln!(out, "            Min          {s_min}").unwrap();
            writeln!(out, "            Median       {s_med}").unwrap();
            writeln!(out, "            Max          {s_max}").unwrap();
            if missing > 0 {
                writeln!(out, "            NumMissing   {missing}").unwrap();
            }
        }
        Array::Duration(dur) if !dur.is_calendar() => {
            let stats = numeric_stats(dur.rows(), dur.cols(), |i| dur.data()[i]);
            if !stats.mins.is_empty() {
                let fmt_dur_row = |vals: &[f64]| -> Result<String> {
                    let mut parts = Vec::with_capacity(vals.len());
                    for &v in vals {
                        parts.push(format_scalar_time(
                            e,
                            &Array::Duration(DurationArray::new(1, 1, vec![v], dur.format())?),
                        )?);
                    }
                    Ok(parts.join("  "))
                };
                writeln!(out, "        Values:").unwrap();
                writeln!(
                    out,
                    "            Min          {}",
                    fmt_dur_row(&stats.mins)?
                )
                .unwrap();
                writeln!(
                    out,
                    "            Median       {}",
                    fmt_dur_row(&stats.meds)?
                )
                .unwrap();
                writeln!(
                    out,
                    "            Max          {}",
                    fmt_dur_row(&stats.maxs)?
                )
                .unwrap();
                writeln!(
                    out,
                    "            Mean         {}",
                    fmt_dur_row(&stats.means)?
                )
                .unwrap();
                writeln!(
                    out,
                    "            Std          {}",
                    fmt_dur_row(&stats.stds)?
                )
                .unwrap();
                let total_miss: f64 = stats.miss.iter().sum();
                if total_miss > 0. {
                    let miss_row = stats
                        .miss
                        .iter()
                        .map(|&v| crate::display_format::scalar_text(v, &st))
                        .collect::<Vec<_>>()
                        .join("  ");
                    writeln!(out, "            NumMissing   {miss_row}").unwrap();
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn write_numeric_stats(out: &mut String, stats: &NumericStats, st: &crate::display_format::Style) {
    if stats.mins.is_empty() {
        return;
    }
    let fmt_row = |vals: &[f64]| -> String {
        vals.iter()
            .map(|&v| crate::display_format::scalar_text(v, st))
            .collect::<Vec<_>>()
            .join("  ")
    };
    writeln!(out, "        Values:").unwrap();
    writeln!(out, "            Min          {}", fmt_row(&stats.mins)).unwrap();
    writeln!(out, "            Median       {}", fmt_row(&stats.meds)).unwrap();
    writeln!(out, "            Max          {}", fmt_row(&stats.maxs)).unwrap();
    writeln!(out, "            Mean         {}", fmt_row(&stats.means)).unwrap();
    writeln!(out, "            Std          {}", fmt_row(&stats.stds)).unwrap();
    let total_miss: f64 = stats.miss.iter().sum();
    if total_miss > 0. {
        writeln!(out, "            NumMissing   {}", fmt_row(&stats.miss)).unwrap();
    }
}

fn format_table(e: &Engine, t: &TableArray) -> Result<String> {
    let mut out = String::new();
    if let Some(rt) = t.row_times() {
        writeln!(out, "RowTimes:").unwrap();
        append_var_summary(e, &mut out, &t.parts().dimension_names[0], rt, "", "")?;
    }
    if t.width() > 0 {
        writeln!(out, "Variables:").unwrap();
        for i in 0..t.width() {
            let desc = t
                .parts()
                .descriptions
                .get(i)
                .map(String::as_str)
                .unwrap_or("");
            let units = t.parts().units.get(i).map(String::as_str).unwrap_or("");
            append_var_summary(e, &mut out, &t.names()[i], &t.columns()[i], desc, units)?;
        }
    }
    Ok(out)
}

fn push_numeric_fields(
    fields: &mut Vec<(String, Arc<Array>)>,
    cols: usize,
    stats: NumericStats,
) -> Result<()> {
    for (label, vals) in [
        ("Min", stats.mins),
        ("Median", stats.meds),
        ("Max", stats.maxs),
        ("Mean", stats.means),
        ("Std", stats.stds),
        ("NumMissing", stats.miss),
    ] {
        fields.push((
            label.into(),
            Arc::new(Array::Numeric(Matrix::from_column_vec(1, cols, vals)?)),
        ));
    }
    Ok(())
}

fn var_struct(e: &Engine, col: &Array, desc: &str, units: &str) -> Result<StructArray> {
    let dims = col.dims();
    let size_vec: Vec<f64> = dims.iter().map(|&d| d as f64).collect();
    let mut fields = vec![
        (
            "Size".to_string(),
            Arc::new(Array::Numeric(Matrix::from_column_vec(
                1,
                size_vec.len(),
                size_vec,
            )?)),
        ),
        (
            "Type".to_string(),
            Arc::new(Array::Character(CharacterRow::from_text(
                class_name(col),
                &e.cancel,
            )?)),
        ),
        (
            "Description".to_string(),
            Arc::new(Array::Character(CharacterRow::from_text(desc, &e.cancel)?)),
        ),
        (
            "Units".to_string(),
            Arc::new(Array::Character(CharacterRow::from_text(units, &e.cancel)?)),
        ),
    ];
    match col {
        Array::Numeric(m) if !m.is_nd() => {
            let stats = numeric_stats(m.rows(), m.cols(), |i| m.data()[i]);
            push_numeric_fields(&mut fields, m.cols(), stats)?;
        }
        Array::Typed(t) if !t.is_nd() => {
            let stats = numeric_stats(t.rows(), t.cols(), |i| t.real(i));
            push_numeric_fields(&mut fields, t.cols(), stats)?;
        }
        Array::Logical(l) => {
            let t_cnt = l.data().iter().filter(|&&b| b != 0).count();
            let f_cnt = l.len() - t_cnt;
            fields.push((
                "True".into(),
                Arc::new(Array::Numeric(Matrix::scalar(t_cnt as f64))),
            ));
            fields.push((
                "False".into(),
                Arc::new(Array::Numeric(Matrix::scalar(f_cnt as f64))),
            ));
        }
        Array::Categorical(c) => {
            let cats = c.categories();
            let mut counts = vec![0.0f64; cats.len()];
            let mut undef = 0usize;
            for &code in c.codes() {
                if code == 0 {
                    undef += 1;
                } else {
                    counts[code as usize - 1] += 1.0;
                }
            }
            let cat_cells: Vec<Arc<Array>> = cats
                .iter()
                .map(|s| {
                    Ok(Arc::new(Array::Character(CharacterRow::from_text(
                        s, &e.cancel,
                    )?)))
                })
                .collect::<Result<Vec<_>>>()?;
            let cell = CellArray::from_shared(cats.len(), 1, cat_cells, &e.cancel)?;
            fields.push(("Categories".into(), Arc::new(Array::Cell(cell))));
            fields.push((
                "Counts".into(),
                Arc::new(Array::Numeric(Matrix::from_column_vec(
                    counts.len(),
                    1,
                    counts,
                )?)),
            ));
            if undef > 0 {
                fields.push((
                    "NumMissing".into(),
                    Arc::new(Array::Numeric(Matrix::scalar(undef as f64))),
                ));
            }
        }
        Array::Datetime(dt) => {
            let (min_v, med_v, max_v, missing) = time_stats(dt.data());
            fields.push((
                "Min".into(),
                Arc::new(Array::Datetime(dt.with_data(1, 1, vec![min_v])?)),
            ));
            fields.push((
                "Median".into(),
                Arc::new(Array::Datetime(dt.with_data(1, 1, vec![med_v])?)),
            ));
            fields.push((
                "Max".into(),
                Arc::new(Array::Datetime(dt.with_data(1, 1, vec![max_v])?)),
            ));
            fields.push((
                "NumMissing".into(),
                Arc::new(Array::Numeric(Matrix::scalar(missing as f64))),
            ));
        }
        Array::Duration(dur) if !dur.is_calendar() => {
            let stats = numeric_stats(dur.rows(), dur.cols(), |i| dur.data()[i]);
            let cols = dur.cols();
            fields.push((
                "Min".into(),
                Arc::new(Array::Duration(DurationArray::new(
                    1,
                    cols,
                    stats.mins,
                    dur.format(),
                )?)),
            ));
            fields.push((
                "Median".into(),
                Arc::new(Array::Duration(DurationArray::new(
                    1,
                    cols,
                    stats.meds,
                    dur.format(),
                )?)),
            ));
            fields.push((
                "Max".into(),
                Arc::new(Array::Duration(DurationArray::new(
                    1,
                    cols,
                    stats.maxs,
                    dur.format(),
                )?)),
            ));
            fields.push((
                "Mean".into(),
                Arc::new(Array::Duration(DurationArray::new(
                    1,
                    cols,
                    stats.means,
                    dur.format(),
                )?)),
            ));
            fields.push((
                "Std".into(),
                Arc::new(Array::Duration(DurationArray::new(
                    1,
                    cols,
                    stats.stds,
                    dur.format(),
                )?)),
            ));
            fields.push((
                "NumMissing".into(),
                Arc::new(Array::Numeric(Matrix::from_column_vec(
                    1, cols, stats.miss,
                )?)),
            ));
        }
        _ => {}
    }
    let (names, slots): (Vec<_>, Vec<_>) = fields.into_iter().unzip();
    StructArray::from_slots(1, 1, names, slots, &e.cancel)
}

fn build_struct(e: &Engine, t: &TableArray) -> Result<StructArray> {
    let mut names = Vec::new();
    let mut slots = Vec::new();
    if let Some(rt) = t.row_times() {
        let time_name = &t.parts().dimension_names[0];
        if !t.names().contains(time_name) {
            names.push(time_name.clone());
            slots.push(Arc::new(Array::Struct(var_struct(e, rt, "", "")?)));
        }
    }
    for i in 0..t.width() {
        let desc = t
            .parts()
            .descriptions
            .get(i)
            .map(String::as_str)
            .unwrap_or("");
        let units = t.parts().units.get(i).map(String::as_str).unwrap_or("");
        names.push(t.names()[i].clone());
        slots.push(Arc::new(Array::Struct(var_struct(
            e,
            &t.columns()[i],
            desc,
            units,
        )?)));
    }
    StructArray::from_slots(1, 1, names, slots, &e.cancel)
}
