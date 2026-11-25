//! Operators on datetime and duration values: + - * / ./ with doubles as
//! exact 24-hour days, comparisons (NaT compares false, ~= true), unary
//! minus, transpose, colon ranges and concatenation.
use super::super::{Engine, Value};
use super::{
    CANCEL_STRIDE, CANCELLED, broadcast, calendar, datetime_of, dt_value, du_value, duration_of,
    expand_index, logical, number_matrix, polled, polled_map, text_elements,
};
use crate::datetime_array::{MAX_ELEMENTS, NANOS_PER_DAY};
use crate::syntax::Token;
use crate::{Array, DatetimeArray, DurationArray, Result};

const MS_PER_DAY: f64 = 86_400_000.0;

/// One operand: its kind and shape.
enum Operand<'a> {
    Dt(&'a DatetimeArray),
    Du(&'a DurationArray),
    Num(Vec<f64>, (usize, usize)),
}

impl Operand<'_> {
    fn shape(&self) -> (usize, usize) {
        match self {
            Self::Dt(t) => (t.rows(), t.cols()),
            Self::Du(d) => (d.rows(), d.cols()),
            Self::Num(_, s) => *s,
        }
    }
}

pub(super) fn class_of(v: &Value) -> String {
    match v {
        Value::Matrix(_) => "double".into(),
        Value::Logical(_) => "logical".into(),
        Value::Complex(_) => "double".into(),
        Value::Text(_) => "char".into(),
        Value::QuotedString(_) => "string".into(),
        Value::Container(a) => a.class().into(),
        Value::Void => "void".into(),
    }
}

fn op_text(op: &Token) -> &'static str {
    match op {
        Token::Plus => "+",
        Token::Minus => "-",
        Token::Star => "*",
        Token::DotStar => ".*",
        Token::Slash => "/",
        Token::DotSlash => "./",
        Token::Backslash => "\\",
        Token::Power => "^",
        Token::DotPower => ".^",
        _ => "operator",
    }
}

impl Engine {
    fn dtm_operand<'a>(&self, v: &'a Value) -> Result<Option<Operand<'a>>> {
        Ok(Some(match v {
            Value::Container(Array::Datetime(t)) => Operand::Dt(t),
            Value::Container(Array::Duration(d)) => Operand::Du(d),
            Value::Matrix(_) | Value::Logical(_) | Value::Container(Array::Typed(_)) => {
                let (x, s) = self.dtm_numbers(v, "datetime")?;
                Operand::Num(x, s)
            }
            _ => return Ok(None),
        }))
    }

    /// `a op b` where either side is a datetime or duration.
    pub(in crate::engine) fn dtm_binary(
        &mut self,
        op: &Token,
        a: &Value,
        b: &Value,
    ) -> Result<Value> {
        if super::calendar_duration::is_calendar(a) || super::calendar_duration::is_calendar(b) {
            return super::calendar_duration::binary(self, op, a, b);
        }
        let unsupported = || {
            format!(
                "Operator '{}' is not supported for {} and {} operands",
                op_text(op),
                class_of(a),
                class_of(b)
            )
        };
        let (Some(x), Some(y)) = (self.dtm_operand(a)?, self.dtm_operand(b)?) else {
            return Err(unsupported());
        };
        use Operand::{Dt, Du, Num};
        match (op, &x, &y) {
            (Token::Plus, Dt(t), Du(_) | Num(..)) => self.dtm_shift(t, &y, false),
            (Token::Plus, Du(_) | Num(..), Dt(t)) => self.dtm_shift(t, &x, false),
            (Token::Minus, Dt(t), Du(_) | Num(..)) => self.dtm_shift(t, &y, true),
            (Token::Minus, Dt(p), Dt(q)) => {
                if p.zone() != q.zone() {
                    return Err(
                        "Cannot subtract a datetime with a time zone from one without".into(),
                    );
                }
                let shape = broadcast(x.shape(), y.shape(), "minus")?;
                let n = shape.0 * shape.1;
                let mut data = Vec::with_capacity(n);
                for i in 0..n {
                    self.dtm_check(i)?;
                    let u = p.data()[expand_index(i, shape.0, x.shape())];
                    let v = q.data()[expand_index(i, shape.0, y.shape())];
                    data.push(match (u.total_nanos(), v.total_nanos()) {
                        (Some(u), Some(v)) => calendar::nanos_to_ms(u - v),
                        _ => f64::NAN,
                    });
                }
                Ok(du_value(DurationArray::new(
                    shape.0,
                    shape.1,
                    data,
                    super::dur_text::DEFAULT,
                )?))
            }
            (Token::Plus | Token::Minus, Du(_) | Num(..), Du(_) | Num(..)) => {
                let format = match (&x, &y) {
                    (Du(d), _) | (_, Du(d)) => d.format().to_string(),
                    _ => unreachable!("one operand is a duration"),
                };
                let minus = matches!(op, Token::Minus);
                self.dtm_duration_zip(
                    &x,
                    &y,
                    format,
                    true,
                    |u, v| if minus { u - v } else { u + v },
                )
            }
            (Token::Star, Du(d), Num(m, s)) | (Token::Star, Num(m, s), Du(d))
                if d.len() != 1 && m.len() != 1 =>
            {
                // Matrix product with a double matrix, through the dense kernel.
                let dm = crate::Matrix::from_column_vec(d.rows(), d.cols(), d.data().to_vec())?;
                let nm = crate::Matrix::from_column_vec(s.0, s.1, m.clone())?;
                let product = if matches!(x, Du(_)) {
                    dm.mul(&nm)?
                } else {
                    nm.mul(&dm)?
                };
                Ok(du_value(DurationArray::new(
                    product.rows(),
                    product.cols(),
                    product.data().to_vec(),
                    d.format(),
                )?))
            }
            (Token::Star | Token::DotStar, Du(d), Num(..))
            | (Token::Star | Token::DotStar, Num(..), Du(d)) => {
                let format = d.format().to_string();
                self.dtm_duration_zip(&x, &y, format, false, |u, v| u * v)
            }
            (Token::Slash | Token::DotSlash, Du(d), Num(m, _)) => {
                if matches!(op, Token::Slash) && m.len() != 1 {
                    return Err("Use ./ to divide a duration array by a numeric array".into());
                }
                let format = d.format().to_string();
                self.dtm_duration_zip(&x, &y, format, false, |u, v| u / v)
            }
            (Token::Slash | Token::DotSlash, Du(p), Du(q)) => {
                if matches!(op, Token::Slash) && q.len() != 1 {
                    return Err("Use ./ to divide duration arrays elementwise".into());
                }
                let shape = broadcast(x.shape(), y.shape(), "rdivide")?;
                let n = shape.0 * shape.1;
                let data = polled(&self.cancel, n, |i| {
                    p.data()[expand_index(i, shape.0, x.shape())]
                        / q.data()[expand_index(i, shape.0, y.shape())]
                })?;
                number_matrix(shape.0, shape.1, data)
            }
            (Token::Star | Token::DotStar, Du(_), Du(_)) => Err(
                "Durations cannot be multiplied together; multiply a duration by a number".into(),
            ),
            _ => Err(unsupported()),
        }
    }

    /// Elementwise duration result, numbers taken as days.
    fn dtm_duration_zip(
        &self,
        x: &Operand<'_>,
        y: &Operand<'_>,
        format: String,
        additive: bool,
        f: impl Fn(f64, f64) -> f64,
    ) -> Result<Value> {
        let shape = broadcast(x.shape(), y.shape(), "duration")?;
        let n = shape.0 * shape.1;
        let mut data = Vec::with_capacity(n);
        // For * and / a number is a plain factor, for + and - a number of days.
        let scale = |o: &Operand<'_>, i: usize, as_days: bool| -> f64 {
            match o {
                Operand::Du(d) => d.data()[expand_index(i, shape.0, o.shape())],
                Operand::Num(m, s) => {
                    let v = m[expand_index(i, shape.0, *s)];
                    if as_days { v * MS_PER_DAY } else { v }
                }
                Operand::Dt(_) => f64::NAN,
            }
        };
        for i in 0..n {
            self.dtm_check(i)?;
            data.push(f(scale(x, i, additive), scale(y, i, additive)));
        }
        Ok(du_value(DurationArray::new(
            shape.0, shape.1, data, format,
        )?))
    }

    /// datetime `t` shifted by a duration or a number of days, elementwise
    /// with implicit expansion; `negate` subtracts.
    fn dtm_shift(&self, t: &DatetimeArray, by: &Operand<'_>, negate: bool) -> Result<Value> {
        let t_shape = (t.rows(), t.cols());
        let by_shape = by.shape();
        let shape = broadcast(t_shape, by_shape, "plus")?;
        let n = shape.0 * shape.1;
        let mut data = Vec::with_capacity(n);
        for i in 0..n {
            self.dtm_check(i)?;
            let v = t.data()[expand_index(i, shape.0, t_shape)];
            let ns = match by {
                Operand::Du(d) => {
                    calendar::ms_to_nanos(d.data()[expand_index(i, shape.0, by_shape)])?
                }
                Operand::Num(m, s) => calendar::days_to_nanos(m[expand_index(i, shape.0, *s)])?,
                Operand::Dt(_) => None,
            };
            data.push(calendar::shifted(
                v,
                ns.map(|ns| if negate { -ns } else { ns }),
            )?);
        }
        Ok(dt_value(t.with_data(shape.0, shape.1, data)?))
    }

    /// Comparison `a op b` where either side is a datetime or duration.
    pub(in crate::engine) fn dtm_compare(
        &mut self,
        op: &Token,
        a: &Value,
        b: &Value,
    ) -> Result<Value> {
        if matches!(
            op,
            Token::And | Token::Or | Token::ShortAnd | Token::ShortOr
        ) {
            return Err(format!(
                "{} values cannot be used as logical values",
                if datetime_of(a).is_some() || datetime_of(b).is_some() {
                    "datetime"
                } else {
                    "duration"
                }
            ));
        }
        // Text beside a datetime is read with that array's format.
        let converted;
        let (a, b) = match (datetime_of(a), datetime_of(b)) {
            (Some(t), None) if text_elements(b, &self.cancel)?.is_some() => {
                converted = dt_value(self.dtm_text_as_datetime(b, t)?.expect("text checked"));
                (a, &converted)
            }
            (None, Some(t)) if text_elements(a, &self.cancel)?.is_some() => {
                converted = dt_value(self.dtm_text_as_datetime(a, t)?.expect("text checked"));
                (&converted, b)
            }
            _ => (a, b),
        };
        let cmp = |o: std::cmp::Ordering| -> bool {
            use std::cmp::Ordering::*;
            match op {
                Token::Equal => o == Equal,
                Token::NotEqual => o != Equal,
                Token::Less => o == Less,
                Token::LessEqual => o != Greater,
                Token::Greater => o == Greater,
                _ => o != Less,
            }
        };
        let missing = matches!(op, Token::NotEqual);
        if super::calendar_duration::is_calendar(a) || super::calendar_duration::is_calendar(b) {
            return Err("calendarDuration: comparisons are not supported; use isequal".into());
        }
        let incomparable = || {
            format!(
                "Comparison is not defined between {} and {} values",
                class_of(a),
                class_of(b)
            )
        };
        let (Some(x), Some(y)) = (self.dtm_operand(a)?, self.dtm_operand(b)?) else {
            return Err(incomparable());
        };
        let shape = broadcast(x.shape(), y.shape(), "comparison")?;
        let n = shape.0 * shape.1;
        let mut data = Vec::with_capacity(n);
        match (&x, &y) {
            (Operand::Dt(p), Operand::Dt(q)) => {
                if p.zone() != q.zone() {
                    return Err("Cannot compare a datetime with a time zone to one without".into());
                }
                for i in 0..n {
                    self.dtm_check(i)?;
                    let u = p.data()[expand_index(i, shape.0, x.shape())].total_nanos();
                    let v = q.data()[expand_index(i, shape.0, y.shape())].total_nanos();
                    data.push(u8::from(match (u, v) {
                        (Some(u), Some(v)) => cmp(u.cmp(&v)),
                        _ => missing,
                    }));
                }
            }
            (Operand::Du(_) | Operand::Num(..), Operand::Du(_) | Operand::Num(..)) => {
                let ms = |o: &Operand<'_>, i: usize| match o {
                    Operand::Du(d) => d.data()[expand_index(i, shape.0, o.shape())],
                    Operand::Num(m, s) => m[expand_index(i, shape.0, *s)] * MS_PER_DAY,
                    Operand::Dt(_) => f64::NAN,
                };
                for i in 0..n {
                    self.dtm_check(i)?;
                    data.push(u8::from(match ms(&x, i).partial_cmp(&ms(&y, i)) {
                        Some(o) => cmp(o),
                        None => missing,
                    }));
                }
            }
            _ => return Err(incomparable()),
        }
        logical(shape.0, shape.1, data)
    }

    pub(in crate::engine) fn dtm_unary(&self, negative: bool, v: Value) -> Result<Value> {
        if super::calendar_duration::is_calendar(&v) {
            return super::calendar_duration::unary(self, negative, v);
        }
        match &v {
            Value::Container(Array::Duration(d)) if negative => Ok(du_value(d.with_data(
                d.rows(),
                d.cols(),
                polled_map(&self.cancel, d.data(), |x| -x)?,
            )?)),
            Value::Container(Array::Datetime(_)) if negative => {
                Err("Unary minus is not defined for datetime values".into())
            }
            _ => Ok(v),
        }
    }

    pub(in crate::engine) fn dtm_transpose(&self, v: &Value) -> Result<Value> {
        if let Some(d) = super::calendar_duration::of(v) {
            let (r, c) = (d.rows(), d.cols());
            let values = d.calendar_data().unwrap();
            let data = polled(&self.cancel, r * c, |i| values[(i % c) * r + i / c])?;
            return Ok(du_value(DurationArray::new_calendar(c, r, data)?));
        }
        // Output element i (c-by-r, column-major) is source (i / c, i % c).
        match v {
            Value::Container(Array::Datetime(t)) => {
                let (r, c) = (t.rows(), t.cols());
                let data = polled(&self.cancel, r * c, |i| t.data()[(i % c) * r + i / c])?;
                Ok(dt_value(t.with_data(c, r, data)?))
            }
            Value::Container(Array::Duration(d)) => {
                let (r, c) = (d.rows(), d.cols());
                let data = polled(&self.cancel, r * c, |i| d.data()[(i % c) * r + i / c])?;
                Ok(du_value(d.with_data(c, r, data)?))
            }
            _ => Err("Expected a datetime or duration value".into()),
        }
    }

    /// `first:step:last` with datetime or duration operands. The endpoint is
    /// included only when it lands exactly on a step.
    pub(in crate::engine) fn dtm_range(
        &mut self,
        first: Value,
        step: Option<Value>,
        last: Value,
    ) -> Result<Value> {
        if super::calendar_duration::is_calendar(&first)
            || super::calendar_duration::is_calendar(&last)
            || step
                .as_ref()
                .is_some_and(super::calendar_duration::is_calendar)
        {
            return Err("calendarDuration: colon ranges are not supported".into());
        }
        let scalar_ns = |v: &Value| -> Result<Option<i128>> {
            match v {
                Value::Container(Array::Duration(d)) if d.len() == 1 => {
                    calendar::ms_to_nanos(d.data()[0])
                }
                Value::Container(Array::Duration(_)) => {
                    Err("Range step must be a scalar duration".into())
                }
                _ => {
                    let (x, _) = self.dtm_numbers(v, "colon")?;
                    match x.as_slice() {
                        [x] => calendar::days_to_nanos(*x),
                        _ => Err("Range step must be a scalar".into()),
                    }
                }
            }
        };
        let step_ns = match &step {
            None => None,
            Some(s) => Some(scalar_ns(s)?.ok_or("Range step must be finite")?),
        };
        if let (Some(t1), Some(t2)) = (datetime_of(&first), datetime_of(&last)) {
            if t1.len() != 1 || t2.len() != 1 {
                return Err("Range bounds must be scalar datetimes".into());
            }
            if t1.zone() != t2.zone() {
                return Err("Range bounds must both have a time zone or both have none".into());
            }
            let step = step_ns.unwrap_or(i128::from(NANOS_PER_DAY));
            let (Some(a), Some(b)) = (t1.data()[0].total_nanos(), t2.data()[0].total_nanos())
            else {
                return Err("Range bounds must not be NaT".into());
            };
            let count = range_count(a, b, step)?;
            let mut data = Vec::with_capacity(count);
            for k in 0..count {
                self.dtm_check(k)?;
                data.push(calendar::from_total(a + k as i128 * step)?);
            }
            return Ok(dt_value(t1.with_data(1, count, data)?));
        }
        if let (Some(d1), Some(d2)) = (duration_of(&first), duration_of(&last)) {
            if d1.len() != 1 || d2.len() != 1 {
                return Err("Range bounds must be scalar durations".into());
            }
            let step = step_ns.ok_or("Duration ranges need a step: d1:step:d2")?;
            let (Some(a), Some(b)) = (
                calendar::ms_to_nanos(d1.data()[0])?,
                calendar::ms_to_nanos(d2.data()[0])?,
            ) else {
                return Err("Range bounds must be finite".into());
            };
            let count = range_count(a, b, step)?;
            let data = polled(&self.cancel, count, |k| {
                calendar::nanos_to_ms(a + k as i128 * step)
            })?;
            return Ok(du_value(d1.with_data(1, count, data)?));
        }
        Err("Range bounds must both be datetimes or both be durations".into())
    }

    /// `[...]` with a datetime or duration among the values.
    pub(in crate::engine) fn dtm_concat(&mut self, rows: Vec<Vec<Value>>) -> Result<Value> {
        if rows
            .iter()
            .flatten()
            .any(super::calendar_duration::is_calendar)
        {
            return super::calendar_duration::concat(self, rows);
        }
        let all: Vec<&Value> = rows.iter().flatten().collect();
        // Like dense concatenation, 0x0 operands are dropped: they decide
        // neither the format nor the time zone.
        let zero = |v: &&Value| *v.dims() == [0, 0];
        let first_dt = all
            .iter()
            .filter(|v| !zero(v))
            .find_map(|v| datetime_of(v))
            .or_else(|| all.iter().find_map(|v| datetime_of(v)));
        if let Some(like) = first_dt.cloned() {
            let mut grid = Vec::with_capacity(rows.len());
            for row in &rows {
                let mut blocks = Vec::with_capacity(row.len());
                for v in row {
                    let t = match v {
                        Value::Container(Array::Datetime(t)) if t.rows() == 0 && t.cols() == 0 => {
                            continue;
                        }
                        Value::Container(Array::Datetime(t)) => {
                            if t.zone() != like.zone() {
                                return Err(
                                    "Cannot concatenate datetimes with and without a time zone"
                                        .into(),
                                );
                            }
                            t.clone()
                        }
                        v if v.dims().iter().product::<usize>() == 0
                            && !matches!(v, Value::Text(_) | Value::QuotedString(_)) =>
                        {
                            continue;
                        }
                        v => match self.dtm_text_as_datetime(v, &like)? {
                            Some(t) => t,
                            None => {
                                return Err(format!(
                                    "Cannot concatenate a datetime with a {} value",
                                    class_of(v)
                                ));
                            }
                        },
                    };
                    blocks.push((t.rows(), t.cols(), t.data().to_vec()));
                }
                grid.push(blocks);
            }
            let (r, c, data) = join_grid(grid, &self.cancel)?;
            return Ok(dt_value(like.with_data(r, c, data)?));
        }
        let like = all
            .iter()
            .filter(|v| !zero(v))
            .find_map(|v| duration_of(v))
            .or_else(|| all.iter().find_map(|v| duration_of(v)))
            .cloned()
            .ok_or("Expected a datetime or duration value")?;
        let mut grid = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut blocks = Vec::with_capacity(row.len());
            for v in row {
                match v {
                    Value::Container(Array::Duration(d)) => {
                        blocks.push((d.rows(), d.cols(), d.data().to_vec()));
                    }
                    v if v.dims().iter().product::<usize>() == 0 => {}
                    Value::Matrix(_) | Value::Logical(_) | Value::Container(Array::Typed(_)) => {
                        let (x, s) = self.dtm_numbers(v, "duration")?;
                        blocks.push((s.0, s.1, polled_map(&self.cancel, &x, |d| d * MS_PER_DAY)?));
                    }
                    v => {
                        return Err(format!(
                            "Cannot concatenate a duration with a {} value",
                            class_of(v)
                        ));
                    }
                }
            }
            grid.push(blocks);
        }
        let (r, c, data) = join_grid(grid, &self.cancel)?;
        Ok(du_value(like.with_data(r, c, data)?))
    }
}

/// Element count of a range from `a` to `b` (nanoseconds) by `step`.
fn range_count(a: i128, b: i128, step: i128) -> Result<usize> {
    if step == 0 {
        return Err("Range step must be nonzero".into());
    }
    let span = b - a;
    if (span > 0 && step < 0) || (span < 0 && step > 0) {
        return Ok(0);
    }
    let count = span / step + 1;
    if count > MAX_ELEMENTS as i128 {
        return Err("Range exceeds the 16777216-element limit".into());
    }
    Ok(count as usize)
}

/// Rows of blocks joined horizontally, then the rows vertically. 0x0 blocks
/// are dropped like in dense concatenation; `cancel` is polled about every
/// 65536 copied elements.
pub(in crate::engine) fn join_grid<T: Copy>(
    grid: Vec<Vec<(usize, usize, Vec<T>)>>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(usize, usize, Vec<T>)> {
    let mut copied = 0usize;
    let mut poll = |n: usize| -> Result<()> {
        copied += n;
        if copied >= CANCEL_STRIDE {
            copied = 0;
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
        }
        Ok(())
    };
    let mut stripes = Vec::with_capacity(grid.len());
    for mut row in grid {
        row.retain(|b| (b.0, b.1) != (0, 0));
        if row.is_empty() {
            continue;
        }
        let rows = row[0].0;
        if row.iter().any(|b| b.0 != rows) {
            return Err("Dimensions of arrays being concatenated are not consistent.".into());
        }
        let cols: usize = row.iter().map(|b| b.1).sum();
        let mut data = Vec::with_capacity(rows * cols);
        for block in row {
            poll(block.2.len())?;
            data.extend(block.2);
        }
        stripes.push((rows, cols, data));
    }
    if stripes.is_empty() {
        return Ok((0, 0, Vec::new()));
    }
    let cols = stripes[0].1;
    if stripes.iter().any(|s| s.1 != cols) {
        return Err("Dimensions of arrays being concatenated are not consistent.".into());
    }
    let rows: usize = stripes.iter().map(|s| s.0).sum();
    crate::datetime_array::checked_len(rows, cols, "concatenated")?;
    let mut data = Vec::with_capacity(rows * cols);
    for c in 0..cols {
        for s in &stripes {
            poll(s.0)?;
            data.extend_from_slice(&s.2[c * s.0..(c + 1) * s.0]);
        }
    }
    Ok((rows, cols, data))
}
