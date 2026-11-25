//! Dot properties (t.Year, t.Format, t.TimeZone, d.Format), indexing and
//! indexed assignment/deletion of datetime and duration arrays. Every write
//! builds new element storage or new metadata: arrays sharing the old storage
//! (`b = a`) never change.
use super::super::{Engine, Execution, Value};
use super::{
    calendar, checked_datetime_format, checked_duration_format, dt_value, du_value, duration_of,
    number_matrix, option_text, parse_zone, polled, polled_map,
};
use crate::datetime_array::{DateTimeValue, NANOS_PER_DAY};
use crate::syntax::{Expr, PathStep};
use crate::{Array, DatetimeArray, Matrix, Result};

const DATETIME_PROPERTIES: &[&str] = &[
    "Year", "Month", "Day", "Hour", "Minute", "Second", "Format", "TimeZone",
];

fn unknown(name: &str, class: &str) -> String {
    format!("Unrecognized property '{name}' for class '{class}'.")
}

impl Engine {
    /// `t.Name` for a datetime or duration array.
    pub(in crate::engine) fn dtm_field(&self, base: &Array, name: &str) -> Result<Value> {
        match base {
            Array::Datetime(t) => match name {
                "Format" => Ok(Value::Text(t.format().into())),
                "TimeZone" => Ok(Value::Text(t.zone().name().into())),
                "Year" | "Month" | "Day" | "Hour" | "Minute" | "Second" => {
                    let field = DATETIME_PROPERTIES
                        .iter()
                        .position(|p| *p == name)
                        .unwrap_or(0);
                    number_matrix(
                        t.rows(),
                        t.cols(),
                        polled_map(&self.cancel, t.data(), |v| super::component(v, field))?,
                    )
                }
                _ => Err(unknown(name, "datetime")),
            },
            Array::Duration(d) => match name {
                "Format" => Ok(Value::Text(d.format().into())),
                _ => Err(unknown(name, "duration")),
            },
            _ => Err("Expected a datetime or duration array".into()),
        }
    }

    /// `t.Name = value` (and `t.Name(i) = value`); None leaves other paths
    /// (such as `t(i).Name`) to the generic code.
    pub(in crate::engine) fn dtm_assign_path(
        &mut self,
        current: &Array,
        steps: &[PathStep],
        value: &Array,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<Option<Array>> {
        if matches!(current,Array::Duration(d) if d.is_calendar()) {
            return Err("calendarDuration: property assignment is not supported".into());
        }
        let Some((PathStep::Field(name), rest)) = steps.split_first() else {
            return Ok(None);
        };
        let value = if rest.is_empty() {
            value.clone()
        } else {
            let field = self.dtm_field(current, name)?.array(&self.cancel)?;
            self.assign_step(Some(field), rest, value.clone(), deletion, out)?
        };
        let value = Value::from(value);
        Ok(Some(match current {
            Array::Datetime(t) => Array::Datetime(self.dtm_set_datetime(t, name, &value)?),
            Array::Duration(d) => match name.as_str() {
                "Format" => {
                    let f = option_text(&value, &self.cancel).ok_or("Format must be text")?;
                    Array::Duration(d.with_format(checked_duration_format(&f)?))
                }
                _ => return Err(unknown(name, "duration")),
            },
            _ => return Ok(None),
        }))
    }

    fn dtm_set_datetime(
        &self,
        t: &DatetimeArray,
        name: &str,
        value: &Value,
    ) -> Result<DatetimeArray> {
        match name {
            "Format" => {
                let f = option_text(value, &self.cancel).ok_or("Format must be text")?;
                return Ok(t.with_format(checked_datetime_format(&f)?));
            }
            // Relabel only: there is no zone database, so no shift.
            "TimeZone" => {
                let z = option_text(value, &self.cancel).ok_or("TimeZone must be text")?;
                return Ok(t.with_zone(parse_zone(&z, "datetime")?));
            }
            "Year" | "Month" | "Day" | "Hour" | "Minute" | "Second" => {}
            _ => return Err(unknown(name, "datetime")),
        }
        let (x, shape) = self.dtm_numbers(value, "datetime")?;
        if x.len() != 1 && shape != (t.rows(), t.cols()) {
            return Err(format!(
                "The {name} value must be a scalar or match the size of the datetime array"
            ));
        }
        let mut data = Vec::with_capacity(t.len());
        for (i, &v) in t.data().iter().enumerate() {
            self.dtm_check(i)?;
            let new = x[if x.len() == 1 { 0 } else { i }];
            data.push(if v.is_nat() || !new.is_finite() {
                DateTimeValue::NAT
            } else {
                set_component(v, name, new)?
            });
        }
        t.with_data(t.rows(), t.cols(), data)
    }

    /// Indexed assignment `name(args) = value` when the target or the value is
    /// a datetime/duration. The subscripts run through the numeric indexed
    /// assignment on a proxy of element numbers, so growth, deletion, logical
    /// masks and `end` behave exactly as for doubles; the result is mapped
    /// back to elements (new gaps are NaT or a zero duration).
    pub(in crate::engine) fn dtm_assign_indexed(
        &mut self,
        name: &str,
        args: &[Expr],
        value: Value,
        deletion: bool,
        quiet: bool,
        out: &mut Execution,
    ) -> Result<bool> {
        let current = self.array(name);
        if matches!(&current,Some(Array::Duration(d)) if d.is_calendar())
            || super::calendar_duration::is_calendar(&value)
        {
            return Err("calendarDuration: indexed assignment is not supported".into());
        }
        let target_dt = matches!(current, Some(Array::Datetime(_) | Array::Duration(_)));
        if !target_dt && !super::is_dt(&value) {
            return Ok(false);
        }
        if !target_dt && current.is_some() {
            return Err(format!(
                "Cannot assign a {} value into a {} array",
                match &value {
                    Value::Container(a) => a.class(),
                    _ => "datetime",
                },
                current.as_ref().map_or("double", Array::class)
            ));
        }
        if args.len() > 2 {
            return Err(
                if matches!(current, Some(Array::Duration(_))) || duration_of(&value).is_some() {
                    super::DURATION_ND.into()
                } else {
                    super::DATETIME_ND.into()
                },
            );
        }
        // The kind and metadata of the result.
        let base: Array = match (&current, &value) {
            (Some(a), _) => a.clone(),
            (None, Value::Container(Array::Datetime(t))) => {
                Array::Datetime(t.with_data(0, 0, vec![])?)
            }
            (None, Value::Container(Array::Duration(d))) => {
                Array::Duration(d.with_data(0, 0, vec![])?)
            }
            _ => unreachable!("one side is a datetime or duration"),
        };
        // New elements, converted to the target's kind.
        let incoming: Option<Array> = if deletion {
            None
        } else {
            Some(match &base {
                Array::Datetime(t) => match &value {
                    Value::Container(Array::Datetime(v)) => {
                        if v.zone() != t.zone() && !(t.is_empty() && current.is_none()) {
                            return Err(
                                "Cannot assign a datetime with a different time zone setting"
                                    .into(),
                            );
                        }
                        Array::Datetime(v.clone())
                    }
                    // NaN assigns NaT, as the reference allows.
                    Value::Matrix(m) if !m.is_empty() && m.data().iter().all(|x| x.is_nan()) => {
                        Array::Datetime(t.with_data(
                            m.rows(),
                            m.cols(),
                            vec![DateTimeValue::NAT; m.len()],
                        )?)
                    }
                    v => match self.dtm_text_as_datetime(v, t)? {
                        Some(v) => Array::Datetime(v),
                        None => {
                            return Err(format!(
                                "Cannot assign a {} value into a datetime array",
                                match v {
                                    Value::Container(a) => a.class(),
                                    Value::Logical(_) => "logical",
                                    _ => "double",
                                }
                            ));
                        }
                    },
                },
                Array::Duration(d) => match &value {
                    Value::Container(Array::Duration(v)) => Array::Duration(v.clone()),
                    v @ (Value::Matrix(_) | Value::Logical(_)) => {
                        let (x, s) = self.dtm_numbers(v, "duration")?;
                        Array::Duration(d.with_data(
                            s.0,
                            s.1,
                            polled_map(&self.cancel, &x, |v| v * 86_400_000.0)?,
                        )?)
                    }
                    v => {
                        return Err(format!(
                            "Cannot assign a {} value into a duration array",
                            match v {
                                Value::Container(a) => a.class(),
                                _ => "char",
                            }
                        ));
                    }
                },
                _ => unreachable!("base is a datetime or duration"),
            })
        };
        let existing = base.len();
        let proxy = Matrix::from_column_vec(
            base.rows(),
            base.cols(),
            polled(&self.cancel, existing, |k| (k + 1) as f64)?,
        )?;
        let rhs = match &incoming {
            None => Value::Matrix(Matrix::from_column_vec(0, 0, vec![])?),
            Some(a) => Value::Matrix(Matrix::from_column_vec(
                a.rows(),
                a.cols(),
                polled(&self.cancel, a.len(), |k| (existing + 1 + k) as f64)?,
            )?),
        };
        let temp = (0usize..)
            .map(|n| format!("Zq_dtm_{n}"))
            .find(|n| !self.contains_array(n))
            .expect("unbounded candidate names");
        self.variables.insert(temp.clone(), proxy);
        let result = self.assign_indexed_value(&temp, args, rhs, deletion, true, out);
        let mapped = self.variables.remove(&temp);
        self.logical_variables.remove(&temp);
        self.complex_variables.remove(&temp);
        self.container_variables.remove(&temp);
        result?;
        let mapped = mapped.ok_or("Indexed assignment did not produce a result")?;
        let pick = |k: f64| -> usize { if k >= 1.0 { k as usize - 1 } else { usize::MAX } };
        let updated = match (&base, &incoming) {
            (Array::Datetime(t), incoming) => {
                let extra = match incoming {
                    Some(Array::Datetime(v)) => v.data(),
                    _ => &[],
                };
                let data = polled_map(&self.cancel, mapped.data(), |k| {
                    let i = pick(k);
                    if i < existing {
                        t.data()[i]
                    } else {
                        extra
                            .get(i.wrapping_sub(existing))
                            .copied()
                            .unwrap_or(DateTimeValue::NAT)
                    }
                })?;
                Array::Datetime(t.with_data(mapped.rows(), mapped.cols(), data)?)
            }
            (Array::Duration(d), incoming) => {
                let extra = match incoming {
                    Some(Array::Duration(v)) => v.data(),
                    _ => &[],
                };
                let data = polled_map(&self.cancel, mapped.data(), |k| {
                    let i = pick(k);
                    if i < existing {
                        d.data()[i]
                    } else {
                        extra.get(i.wrapping_sub(existing)).copied().unwrap_or(0.0)
                    }
                })?;
                Array::Duration(d.with_data(mapped.rows(), mapped.cols(), data)?)
            }
            _ => unreachable!("base is a datetime or duration"),
        };
        self.assign_workspace_array(name, updated, quiet, out)?;
        Ok(true)
    }

    /// `t(i, j)` selection.
    pub(in crate::engine) fn dtm_select(
        &self,
        a: &Array,
        shape: (usize, usize),
        positions: &[usize],
    ) -> Result<Value> {
        match a {
            Array::Datetime(t) => Ok(dt_value(t.select(shape.0, shape.1, positions)?)),
            Array::Duration(d) => Ok(du_value(d.select(shape.0, shape.1, positions)?)),
            _ => Err("Expected a datetime or duration array".into()),
        }
    }
}

/// `v` with one calendar field replaced; the other fields keep their exact
/// values and the result carries (month 13, second 75, ...).
fn set_component(v: DateTimeValue, name: &str, new: f64) -> Result<DateTimeValue> {
    let p = calendar::parts(v);
    let (mut y, mut mo, mut d) = (i128::from(p.y), p.mo as i128, i128::from(p.d));
    let mut hour_ns = i128::from(p.h) * 3_600_000_000_000;
    let mut minute_ns = i128::from(p.mi) * 60_000_000_000;
    let mut second_ns = i128::from(p.s) * 1_000_000_000 + i128::from(p.ns);
    let mut day_frac_ns = 0;
    let whole = |x: f64| -> Result<i128> {
        if x.fract() != 0.0 || x.abs() >= 1e15 {
            Err(format!(
                "datetime: {name} must be an integer below 1e15 in magnitude"
            ))
        } else {
            Ok(x as i128)
        }
    };
    match name {
        "Year" => y = whole(new)?,
        "Month" => mo = whole(new)?,
        "Day" => {
            d = whole(new.trunc())?;
            day_frac_ns = calendar::scaled_nanos(new.fract(), i128::from(NANOS_PER_DAY))?;
        }
        "Hour" => hour_ns = calendar::scaled_nanos(new, 3_600_000_000_000)?,
        "Minute" => minute_ns = calendar::scaled_nanos(new, 60_000_000_000)?,
        _ => second_ns = calendar::scaled_nanos(new, 1_000_000_000)?,
    }
    calendar::from_fields(y, mo, d, hour_ns + minute_ns + second_ns + day_frac_ns)
}
