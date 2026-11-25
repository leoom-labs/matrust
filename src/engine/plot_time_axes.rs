//! Engine-side conversions between datetime/duration/categorical arrays and
//! internal plot coordinates, limits, ticks, and ruler properties.

use super::{Engine, Value};
use crate::datetime_array::{DEFAULT_FORMAT as DT_DEFAULT_FORMAT, TimeZone};
use crate::duration_array::DEFAULT_FORMAT as DUR_DEFAULT_FORMAT;
use crate::plot::{Axes, RulerKind, time_axes};
use crate::{Array, CategoricalArray, DatetimeArray, DurationArray, Matrix, Result};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub(crate) fn is_plot_data_arg(v: &Value) -> bool {
    matches!(
        v,
        Value::Matrix(_)
            | Value::Complex(_)
            | Value::Logical(_)
            | Value::Container(
                Array::Datetime(_) | Array::Duration(_) | Array::Categorical(_) | Array::Typed(_)
            )
    )
}

pub(crate) fn is_non_numeric_data(v: &Value) -> bool {
    matches!(
        v,
        Value::Container(Array::Datetime(_) | Array::Duration(_) | Array::Categorical(_))
    )
}

/// Extract plot coordinates `(Matrix, RulerKind)` from a `Value`, validating
/// against `existing` when adding to a held axes or an earlier series in the
/// same call.
pub(crate) fn extract_axis_data(
    v: &Value,
    existing: Option<&RulerKind>,
    axis_name: &str,
    cancel: &AtomicBool,
) -> Result<(Matrix, RulerKind)> {
    match v {
        Value::Container(Array::Datetime(dt)) => {
            if let Some(ex) = existing
                && !matches!(ex, RulerKind::Datetime { .. })
            {
                return Err(format!(
                    "Values plotted against {axis_name}-axis must be {} values",
                    ex.kind_word()
                ));
            }
            let kind = match existing {
                Some(RulerKind::Datetime { utc, format }) => RulerKind::Datetime {
                    utc: *utc,
                    format: format.clone(),
                },
                _ => RulerKind::Datetime {
                    utc: dt.zone() == TimeZone::Utc,
                    format: dt.format().to_owned(),
                },
            };
            let mut coords = Vec::with_capacity(dt.len());
            for (i, &val) in dt.data().iter().enumerate() {
                if i.is_multiple_of(4096) && cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("Calculation cancelled".into());
                }
                coords.push(time_axes::datetime_to_plot(val));
            }
            Ok((Matrix::from_column_vec(dt.rows(), dt.cols(), coords)?, kind))
        }
        Value::Container(Array::Duration(dur)) => {
            if dur.is_calendar() {
                return Err(
                    "calendarDuration values cannot be plotted directly; convert with days, hours, or seconds"
                        .into(),
                );
            }
            if let Some(ex) = existing
                && !matches!(ex, RulerKind::Duration { .. })
            {
                return Err(format!(
                    "Values plotted against {axis_name}-axis must be {} values",
                    ex.kind_word()
                ));
            }
            let (format, unit_ms) = match existing {
                Some(RulerKind::Duration { format, unit_ms }) => (format.clone(), *unit_ms),
                _ => (
                    dur.format().to_owned(),
                    time_axes::duration_unit_ms(dur.format()),
                ),
            };
            let mut coords = Vec::with_capacity(dur.len());
            for (i, &ms) in dur.data().iter().enumerate() {
                if i.is_multiple_of(4096) && cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("Calculation cancelled".into());
                }
                coords.push(ms / unit_ms);
            }
            Ok((
                Matrix::from_column_vec(dur.rows(), dur.cols(), coords)?,
                RulerKind::Duration { format, unit_ms },
            ))
        }
        Value::Container(Array::Categorical(cat)) => {
            if let Some(ex) = existing
                && !matches!(ex, RulerKind::Categorical { .. })
            {
                return Err(format!(
                    "Values plotted against {axis_name}-axis must be {} values",
                    ex.kind_word()
                ));
            }
            let (mut merged, ordinal) = match existing {
                Some(RulerKind::Categorical {
                    categories,
                    ordinal,
                }) => {
                    if (*ordinal || cat.is_ordinal())
                        && (categories.as_slice() != cat.categories().as_slice()
                            || *ordinal != cat.is_ordinal())
                    {
                        return Err(
                            "Ordinal categorical values plotted on the same axis must have the same categories"
                                .into(),
                        );
                    }
                    (categories.clone(), *ordinal)
                }
                _ => (cat.categories().as_ref().clone(), cat.is_ordinal()),
            };
            let mut coords = Vec::with_capacity(cat.len());
            for (i, &code) in cat.codes().iter().enumerate() {
                if i.is_multiple_of(4096) && cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("Calculation cancelled".into());
                }
                if code == 0 {
                    coords.push(f64::NAN);
                } else {
                    let name = &cat.categories()[code as usize - 1];
                    let idx = if let Some(pos) = merged.iter().position(|c| c == name) {
                        pos + 1
                    } else {
                        merged.push(name.clone());
                        merged.len()
                    };
                    coords.push(idx as f64);
                }
            }
            Ok((
                Matrix::from_column_vec(cat.rows(), cat.cols(), coords)?,
                RulerKind::Categorical {
                    categories: merged,
                    ordinal,
                },
            ))
        }
        _ => {
            if let Some(ex) = existing
                && !ex.is_numeric()
            {
                return Err(format!(
                    "Values plotted against {axis_name}-axis must be {} values",
                    ex.kind_word()
                ));
            }
            let m = v.clone().matrix(cancel)?;
            Ok((m, RulerKind::Numeric))
        }
    }
}

/// Sort `(x, y)` by `x` for `bar(categorical, y)` so bars appear in category order.
pub(crate) fn sort_bar_categorical(x: Matrix, y: Matrix) -> Result<(Matrix, Matrix)> {
    if x.rows() != 1 && x.cols() != 1 {
        return Err("Bar X must be a vector".into());
    }
    let n = x.len();
    if n <= 1 {
        return Ok((x, y));
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| x.data()[a].total_cmp(&x.data()[b]));
    let sorted_x: Vec<f64> = order.iter().map(|&i| x.data()[i]).collect();
    let x_out = Matrix::from_column_vec(x.rows(), x.cols(), sorted_x)?;
    let y_out = if y.rows() == n {
        let mut out = Vec::with_capacity(y.len());
        for c in 0..y.cols() {
            for &r in &order {
                out.push(y.get(r, c));
            }
        }
        Matrix::from_column_vec(y.rows(), y.cols(), out)?
    } else if y.cols() == n && y.rows() > 1 {
        let mut out = Vec::with_capacity(y.len());
        for &c in &order {
            for r in 0..y.rows() {
                out.push(y.get(r, c));
            }
        }
        Matrix::from_column_vec(y.rows(), y.cols(), out)?
    } else if y.rows() == 1 && y.cols() == n {
        let sorted_y: Vec<f64> = order.iter().map(|&i| y.data()[i]).collect();
        Matrix::from_column_vec(y.rows(), y.cols(), sorted_y)?
    } else {
        return Err("Bar X and Y dimensions must match".into());
    };
    Ok((x_out, y_out))
}

/// Convert internal axis coordinates (limits or ticks) to the MATLAB `Value`
/// matching `kind`.
pub(crate) fn ruler_values_to_value(
    kind: &RulerKind,
    values: &[f64],
    _cancel: &AtomicBool,
) -> Result<Value> {
    match kind {
        RulerKind::Numeric => Ok(Value::Matrix(Matrix::from_column_slice(
            1,
            values.len(),
            values,
        )?)),
        RulerKind::Datetime { utc, format } => {
            let data = values
                .iter()
                .map(|&d| time_axes::plot_to_datetime(d))
                .collect();
            let fmt = if format.is_empty() {
                DT_DEFAULT_FORMAT
            } else {
                format.as_str()
            };
            let zone = if *utc {
                TimeZone::Utc
            } else {
                TimeZone::Unzoned
            };
            Ok(Value::Container(Array::Datetime(DatetimeArray::new(
                1,
                values.len(),
                data,
                fmt,
                zone,
            )?)))
        }
        RulerKind::Duration { format, unit_ms } => {
            let data = values.iter().map(|&v| v * unit_ms).collect();
            let fmt = if format.is_empty() {
                DUR_DEFAULT_FORMAT
            } else {
                format.as_str()
            };
            Ok(Value::Container(Array::Duration(DurationArray::new(
                1,
                values.len(),
                data,
                fmt,
            )?)))
        }
        RulerKind::Categorical {
            categories,
            ordinal,
        } => {
            let codes = if categories.is_empty() {
                vec![0; values.len()]
            } else {
                values
                    .iter()
                    .map(|&v| {
                        if v.is_finite() {
                            v.round().clamp(1.0, categories.len() as f64) as u32
                        } else {
                            0
                        }
                    })
                    .collect()
            };
            Ok(Value::Container(Array::Categorical(CategoricalArray::new(
                1,
                values.len(),
                codes,
                Arc::new(categories.clone()),
                *ordinal,
            )?)))
        }
    }
}

/// Parse user-supplied limits or ticks `Value` into internal `f64` coordinates
/// for `kind`.
pub(crate) fn parse_ruler_values(
    kind: &RulerKind,
    value: &Value,
    what: &str,
    cancel: &AtomicBool,
) -> Result<Vec<f64>> {
    match kind {
        RulerKind::Numeric => {
            let m = value.clone().matrix(cancel)?;
            if !m.is_empty() && m.rows() != 1 && m.cols() != 1 {
                return Err(format!("{what} must be a vector"));
            }
            Ok(m.data().to_vec())
        }
        RulerKind::Datetime { .. } => {
            let Value::Container(Array::Datetime(dt)) = value else {
                return Err(format!("{what} must be datetime values"));
            };
            if !dt.is_empty() && dt.rows() != 1 && dt.cols() != 1 {
                return Err(format!("{what} must be a vector"));
            }
            Ok(dt
                .data()
                .iter()
                .map(|&v| time_axes::datetime_to_plot(v))
                .collect())
        }
        RulerKind::Duration { unit_ms, .. } => {
            let Value::Container(Array::Duration(dur)) = value else {
                return Err(format!("{what} must be duration values"));
            };
            if dur.is_calendar() {
                return Err(format!("{what} must be fixed duration values"));
            }
            if !dur.is_empty() && dur.rows() != 1 && dur.cols() != 1 {
                return Err(format!("{what} must be a vector"));
            }
            Ok(dur.data().iter().map(|&ms| ms / unit_ms).collect())
        }
        RulerKind::Categorical { categories, .. } => match value {
            Value::Container(Array::Categorical(cat)) => {
                if !cat.is_empty() && cat.rows() != 1 && cat.cols() != 1 {
                    return Err(format!("{what} must be a vector"));
                }
                let mut out = Vec::with_capacity(cat.len());
                for &code in cat.codes() {
                    if code == 0 {
                        return Err(format!("{what} cannot contain <undefined> categories"));
                    }
                    let name = &cat.categories()[code as usize - 1];
                    let idx = categories
                        .iter()
                        .position(|c| c == name)
                        .ok_or_else(|| format!("Category '{name}' is not in this axis"))?;
                    out.push((idx + 1) as f64);
                }
                Ok(out)
            }
            Value::Container(Array::Str(sa)) => {
                let mut out = Vec::with_capacity(sa.len());
                for i in 0..sa.len() {
                    let name = sa.data()[i]
                        .as_deref()
                        .ok_or_else(|| format!("{what} cannot contain missing strings"))?;
                    let idx = categories
                        .iter()
                        .position(|c| c == name)
                        .ok_or_else(|| format!("Category '{name}' is not in this axis"))?;
                    out.push((idx + 1) as f64);
                }
                Ok(out)
            }
            _ => Err(format!("{what} must be categorical values")),
        },
    }
}

pub(crate) fn existing_ruler_for_plot(e: &Engine, axis: usize) -> Option<&RulerKind> {
    e.figure
        .as_ref()
        .filter(|f| {
            f.hold && (!f.axes.series.is_empty() || !f.axes.extras.rulers[axis].kind.is_numeric())
        })
        .map(|f| &f.axes.extras.rulers[axis].kind)
}

pub(crate) fn validate_tick_format(kind: &RulerKind, fmt: &str) -> Result<()> {
    match kind {
        RulerKind::Numeric => Ok(()),
        RulerKind::Datetime { .. } => {
            super::datetime_fns::text::tokens(fmt)?;
            Ok(())
        }
        RulerKind::Duration { .. } => {
            super::datetime_fns::dur_text::parse_format(fmt)?;
            Ok(())
        }
        RulerKind::Categorical { .. } => {
            Err("CategoricalRuler does not support TickLabelFormat".into())
        }
    }
}

pub(crate) fn ruler_categories_value(
    axes: &Axes,
    axis: usize,
    cancel: &AtomicBool,
) -> Result<Value> {
    let RulerKind::Categorical { categories, .. } = &axes.extras.rulers[axis].kind else {
        return Err("Categories is only available on CategoricalRuler".into());
    };
    super::graphics_ticks::labels_to_cell(categories, cancel)
}
