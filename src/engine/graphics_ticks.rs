//! Tick, tick-label, tick-angle, and colororder commands.
use super::graphics_handles::dot_syntax::{apply_color_order_rows, handle_value};
use super::{Engine, Value};
use crate::plot::{Axes, AxisRuler, MAX_COLOR_ORDER_ROWS, MAX_TICKS, colormap};
use crate::{Array, CellArray, CharacterRow, GraphicsHandle, GraphicsKind, Matrix, Result};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub(super) fn is_tick_or_colororder(name: &str) -> bool {
    matches!(
        name,
        "xticks"
            | "yticks"
            | "zticks"
            | "xticklabels"
            | "yticklabels"
            | "zticklabels"
            | "xtickangle"
            | "ytickangle"
            | "ztickangle"
            | "xtickformat"
            | "ytickformat"
            | "ztickformat"
            | "colororder"
    )
}

pub(crate) fn labels_to_cell(labels: &[String], cancel: &AtomicBool) -> Result<Value> {
    let slots = labels
        .iter()
        .map(|s| CharacterRow::from_text(s, cancel).map(|c| Arc::new(Array::Character(c))))
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        labels.len(),
        1,
        slots,
        cancel,
    )?)))
}

pub(crate) fn parse_tick_labels(value: &Value, cancel: &AtomicBool) -> Result<Vec<String>> {
    match value {
        Value::Text(s) => Ok(vec![s.clone()]),
        Value::Matrix(m) if m.is_empty() => Ok(Vec::new()),
        Value::Container(Array::Character(ch)) => {
            if ch.rows() > MAX_TICKS {
                return Err(format!("TickLabels exceed {MAX_TICKS} entries"));
            }
            if ch.rows() == 0 {
                return Ok(Vec::new());
            }
            let units = ch.units();
            let rows = ch.rows();
            let mut out = Vec::with_capacity(rows);
            for r in 0..rows {
                if r.is_multiple_of(256) && cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
                let row_units: Vec<u16> = (0..ch.cols()).map(|c| units[c * rows + r]).collect();
                let s = String::from_utf16_lossy(&row_units);
                out.push(s.trim_end_matches(' ').to_owned());
            }
            crate::plot::validate_tick_labels(&out)?;
            Ok(out)
        }
        Value::Container(Array::Str(sa)) => {
            if sa.len() > MAX_TICKS {
                return Err(format!("TickLabels exceed {MAX_TICKS} entries"));
            }
            let mut out = Vec::with_capacity(sa.len());
            for i in 0..sa.len() {
                let s = sa.data()[i]
                    .as_deref()
                    .ok_or("TickLabels string array cannot contain missing values")?;
                out.push(s.to_owned());
            }
            crate::plot::validate_tick_labels(&out)?;
            Ok(out)
        }
        Value::Container(Array::Cell(cell)) => {
            if cell.len() > MAX_TICKS {
                return Err(format!("TickLabels exceed {MAX_TICKS} entries"));
            }
            let mut out = Vec::with_capacity(cell.len());
            for i in 0..cell.len() {
                let elem = cell.get_linear(i)?;
                let text = match elem.as_ref() {
                    Array::Character(ch) => {
                        if ch.rows() > 1 {
                            return Err("TickLabels cell elements must be single-line text".into());
                        }
                        String::from_utf16_lossy(ch.units())
                    }
                    Array::Str(sa) if sa.len() == 1 => sa.data()[0]
                        .as_deref()
                        .ok_or("TickLabels string cannot be missing")?
                        .to_owned(),
                    _ => return Err("TickLabels cell elements must be character text".into()),
                };
                out.push(text);
            }
            crate::plot::validate_tick_labels(&out)?;
            Ok(out)
        }
        _ => Err("TickLabels must be a cellstr, string array, or char matrix".into()),
    }
}

pub(crate) fn colororder_from_value(value: &Value, cancel: &AtomicBool) -> Result<Vec<[f64; 3]>> {
    match value {
        Value::Text(name) => named_color_order(name),
        Value::Container(Array::Str(sa)) if sa.len() == 1 => {
            let name = sa.data()[0]
                .as_deref()
                .ok_or("ColorOrder name cannot be missing")?;
            named_color_order(name)
        }
        _ => {
            let m = value.clone().matrix(cancel)?;
            if m.cols() != 3 || m.rows() == 0 || m.rows() > MAX_COLOR_ORDER_ROWS {
                return Err(format!(
                    "ColorOrder requires a real n-by-3 matrix with 1..{MAX_COLOR_ORDER_ROWS} rows"
                ));
            }
            let rows: Vec<[f64; 3]> = (0..m.rows())
                .map(|r| [m.get(r, 0), m.get(r, 1), m.get(r, 2)])
                .collect();
            if rows
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
            {
                return Err("ColorOrder values must be in [0,1]".into());
            }
            Ok(rows)
        }
    }
}

fn named_color_order(name: &str) -> Result<Vec<[f64; 3]>> {
    let lower = name.to_ascii_lowercase();
    if lower == "gem" || lower == "default" {
        return Ok(crate::plot::default_color_order());
    }
    if let Some(rows) = colormap::named(&lower, 7)? {
        return Ok(rows);
    }
    Err(format!("Unknown colororder palette '{name}'"))
}

pub(crate) fn effective_color_order_matrix(
    axes: &Axes,
    figure: &crate::plot::Figure,
    theme_host: crate::plot::BaseTheme,
) -> Result<Value> {
    let rows: Vec<[f64; 3]> = if let Some(cycle) = axes.palette_cycle
        && axes.extras.color_order.is_none()
    {
        let theme = figure.resolve_appearance(theme_host).theme;
        super::graphics_root::automatic_color_order(cycle.version, theme).to_vec()
    } else {
        axes.extras.effective_color_order().as_ref().clone()
    };
    let mut data = Vec::with_capacity(rows.len() * 3);
    for c in 0..3 {
        data.extend(rows.iter().map(|r| r[c]));
    }
    Ok(Value::Matrix(Matrix::from_column_vec(rows.len(), 3, data)?))
}

impl Engine {
    pub(super) fn graphics_ticks_or_colororder(
        &mut self,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value> {
        if name == "colororder" {
            return self.colororder_call(args);
        }
        let axis = match name {
            "xticks" | "xticklabels" | "xtickangle" | "xtickformat" => 0,
            "yticks" | "yticklabels" | "ytickangle" | "ytickformat" => 1,
            "zticks" | "zticklabels" | "ztickangle" | "ztickformat" => 2,
            _ => unreachable!(),
        };
        let mut figure = self.figure.clone().unwrap_or_default();
        if self.figure.is_none() {
            self.apply_figure_defaults(&mut figure)?;
        }
        figure.activate_tile();
        let axes = &mut figure.axes;
        match name {
            "xticks" | "yticks" | "zticks" => match args.as_slice() {
                [] => {
                    let (ticks, _) = axes.effective_ticks(axis);
                    return super::plot_time_axes::ruler_values_to_value(
                        &axes.extras.rulers[axis].kind,
                        &ticks,
                        &self.cancel,
                    );
                }
                [Value::Text(opt)] if opt.eq_ignore_ascii_case("auto") => {
                    axes.extras.rulers[axis].ticks = None;
                }
                [Value::Text(opt)] if opt.eq_ignore_ascii_case("manual") => {
                    let (ticks, _) = axes.effective_ticks(axis);
                    axes.extras.rulers[axis].ticks = Some(ticks);
                }
                [Value::Text(opt)] if opt.eq_ignore_ascii_case("mode") => {
                    return Ok(Value::Text(
                        if axes.extras.rulers[axis].ticks.is_some() {
                            "manual"
                        } else {
                            "auto"
                        }
                        .into(),
                    ));
                }
                [value] => {
                    let ticks = super::plot_time_axes::parse_ruler_values(
                        &axes.extras.rulers[axis].kind,
                        value,
                        "Ticks",
                        &self.cancel,
                    )?;
                    crate::plot::validate_ticks(&ticks)?;
                    axes.extras.rulers[axis].ticks = Some(ticks);
                }
                _ => return Err(format!("{name} takes zero or one argument")),
            },
            "xticklabels" | "yticklabels" | "zticklabels" => match args.as_slice() {
                [] => {
                    let labels = axes.effective_tick_labels(axis);
                    return labels_to_cell(&labels, &self.cancel);
                }
                [Value::Text(opt)] if opt.eq_ignore_ascii_case("auto") => {
                    axes.extras.rulers[axis].tick_labels = None;
                }
                [Value::Text(opt)] if opt.eq_ignore_ascii_case("manual") => {
                    let labels = axes.effective_tick_labels(axis);
                    axes.extras.rulers[axis].tick_labels = Some(labels);
                }
                [Value::Text(opt)] if opt.eq_ignore_ascii_case("mode") => {
                    return Ok(Value::Text(
                        if axes.extras.rulers[axis].tick_labels.is_some() {
                            "manual"
                        } else {
                            "auto"
                        }
                        .into(),
                    ));
                }
                [value] => {
                    let labels = parse_tick_labels(value, &self.cancel)?;
                    axes.extras.rulers[axis].tick_labels = Some(labels);
                }
                _ => return Err(format!("{name} takes zero or one argument")),
            },
            "xtickangle" | "ytickangle" | "ztickangle" => match args.as_slice() {
                [] => {
                    return Ok(Value::Matrix(Matrix::scalar(
                        axes.extras.rulers[axis].tick_label_rotation,
                    )));
                }
                [value] => {
                    let deg = value.clone().matrix(&self.cancel)?.as_scalar()?;
                    if !deg.is_finite() {
                        return Err("TickLabelRotation must be finite".into());
                    }
                    axes.extras.rulers[axis].tick_label_rotation = deg;
                }
                _ => return Err(format!("{name} takes zero or one argument")),
            },
            "xtickformat" | "ytickformat" | "ztickformat" => match args.as_slice() {
                [] => {
                    return ruler_get_property(
                        &figure.axes,
                        &figure,
                        axis,
                        "ticklabelformat",
                        self.graphics_theme_context,
                        &self.cancel,
                    );
                }
                [value] => {
                    let fmt = super::graphics_handles::text(value)?;
                    if fmt.eq_ignore_ascii_case("auto") || fmt.is_empty() {
                        axes.extras.rulers[axis].tick_label_format = None;
                    } else {
                        super::plot_time_axes::validate_tick_format(
                            &axes.extras.rulers[axis].kind,
                            fmt,
                        )?;
                        axes.extras.rulers[axis].tick_label_format = Some(fmt.to_owned());
                    }
                }
                _ => return Err(format!("{name} takes zero or one argument")),
            },
            _ => unreachable!(),
        }
        figure.prepare_legend_layout()?;
        figure.validate()?;
        self.figure = Some(figure);
        Ok(Value::Void)
    }

    fn colororder_call(&mut self, args: Vec<Value>) -> Result<Value> {
        let (target, rest) = if let Some(first) = args.first()
            && let Ok(h) = super::graphics_handles::handle(first)
        {
            (Some(h.clone()), &args[1..])
        } else {
            (None, args.as_slice())
        };
        match (target, rest) {
            (None, []) => {
                let figure = self.figure.clone().unwrap_or_default();
                effective_color_order_matrix(&figure.axes, &figure, self.graphics_theme_context)
            }
            (Some(h), []) => {
                let located = self.locate_graphics(&h)?;
                let figure = self.graphics_figure(located.figure)?;
                let axes = match h.kind() {
                    GraphicsKind::Figure => &figure.axes,
                    GraphicsKind::Axes => {
                        let pos = located.position.unwrap();
                        figure
                            .all_axes()
                            .find(|(p, _)| *p == pos)
                            .map(|(_, a)| a)
                            .ok_or("Axes handle is no longer valid")?
                    }
                    _ => return Err("colororder target must be a Figure or Axes handle".into()),
                };
                effective_color_order_matrix(axes, figure, self.graphics_theme_context)
            }
            (None, [map_val]) => {
                let rows = colororder_from_value(map_val, &self.cancel)?;
                let mut figure = self.figure.clone().unwrap_or_default();
                if self.figure.is_none() {
                    self.apply_figure_defaults(&mut figure)?;
                }
                figure.activate_tile();
                apply_color_order_rows(&mut figure.axes, rows);
                figure.validate()?;
                self.figure = Some(figure);
                Ok(Value::Void)
            }
            (Some(h), [map_val]) => {
                let rows = colororder_from_value(map_val, &self.cancel)?;
                let located = self.locate_graphics(&h)?;
                let mut figure = self.graphics_figure(located.figure)?.clone();
                match h.kind() {
                    GraphicsKind::Figure => {
                        for ax in figure.all_axes_mut() {
                            apply_color_order_rows(ax, rows.clone());
                        }
                    }
                    GraphicsKind::Axes => {
                        let ax = figure
                            .all_axes_mut()
                            .find(|a| a.identity.same_object(&h.object))
                            .ok_or("Axes handle is no longer valid")?;
                        apply_color_order_rows(ax, rows);
                    }
                    _ => return Err("colororder target must be a Figure or Axes handle".into()),
                }
                figure.validate()?;
                self.publish_graphics_figure(located.figure, figure);
                Ok(Value::Void)
            }
            _ => Err("colororder expects 0, 1, or 2 arguments".into()),
        }
    }
}

pub(crate) fn ruler_get_property(
    axes: &Axes,
    figure: &crate::plot::Figure,
    axis: usize,
    name: &str,
    theme_host: crate::plot::BaseTheme,
    cancel: &AtomicBool,
) -> Result<Value> {
    let ruler: &AxisRuler = &axes.extras.rulers[axis];
    match name {
        "type" => Ok(Value::Text("numericruler".into())),
        "parent" => handle_value(
            GraphicsHandle::axes(&figure.identity, &axes.identity),
            cancel,
        ),
        "tickvalues" => {
            let (ticks, _) = axes.effective_ticks(axis);
            super::plot_time_axes::ruler_values_to_value(&ruler.kind, &ticks, cancel)
        }
        "tickvaluesmode" => Ok(Value::Text(
            if ruler.ticks.is_some() {
                "manual"
            } else {
                "auto"
            }
            .into(),
        )),
        "ticklabels" => {
            let labels = match &ruler.tick_labels {
                Some(labels) => labels.clone(),
                None => axes.effective_tick_labels(axis),
            };
            labels_to_cell(&labels, cancel)
        }
        "ticklabelsmode" => Ok(Value::Text(
            if ruler.tick_labels.is_some() {
                "manual"
            } else {
                "auto"
            }
            .into(),
        )),
        "ticklabelrotation" => Ok(Value::Matrix(Matrix::scalar(ruler.tick_label_rotation))),
        "limits" => {
            let lim = match axis {
                0 => axes.bounds().0,
                1 => axes.bounds().1,
                _ => axes.zbounds(),
            };
            super::plot_time_axes::ruler_values_to_value(&ruler.kind, &lim, cancel)
        }
        "limitsmode" => {
            let is_manual = match axis {
                0 => axes.xlim.is_some(),
                1 => axes.ylim.is_some(),
                _ => axes.zlim.is_some(),
            };
            Ok(Value::Text(
                if is_manual { "manual" } else { "auto" }.into(),
            ))
        }
        "exponent" => Ok(Value::Matrix(Matrix::scalar(f64::from(ruler.exponent)))),
        "ticklabelformat" => {
            let text = match &ruler.kind {
                crate::plot::RulerKind::Numeric => ruler
                    .tick_label_format
                    .clone()
                    .unwrap_or_else(|| "%g".into()),
                crate::plot::RulerKind::Datetime { format, .. } => {
                    ruler.tick_label_format.clone().unwrap_or_else(|| {
                        if format.is_empty() {
                            crate::datetime_array::DEFAULT_FORMAT.into()
                        } else {
                            format.clone()
                        }
                    })
                }
                crate::plot::RulerKind::Duration { format, .. } => {
                    ruler.tick_label_format.clone().unwrap_or_else(|| {
                        if format.is_empty() {
                            crate::duration_array::DEFAULT_FORMAT.into()
                        } else {
                            format.clone()
                        }
                    })
                }
                crate::plot::RulerKind::Categorical { .. } => {
                    return Err("CategoricalRuler does not have TickLabelFormat".into());
                }
            };
            Ok(Value::Text(text))
        }
        "categories" => super::plot_time_axes::ruler_categories_value(axes, axis, cancel),
        "color" => {
            let appearance = figure.resolve_appearance(theme_host);
            let colors = axes.resolve_colors(appearance);
            let rgb = match axis {
                0 => colors.x,
                1 => colors.y,
                _ => axes
                    .colors
                    .and_then(|c| c.z)
                    .map_or(appearance.axis, crate::plot::AxesColors::rgb8),
            };
            let norm = rgb.map(|c| f64::from(c) / 255.);
            Ok(Value::Matrix(Matrix::from_column_slice(1, 3, &norm)?))
        }
        "label" => {
            let id = match axis {
                0 => &axes.extras.xlabel_identity,
                1 => &axes.extras.ylabel_identity,
                _ => &axes.extras.zlabel_identity,
            };
            handle_value(
                GraphicsHandle::axes_object(
                    &figure.identity,
                    &axes.identity,
                    id,
                    GraphicsKind::Text,
                ),
                cancel,
            )
        }
        _ => Err(format!("Unsupported NumericRuler property '{name}'")),
    }
}

pub(crate) fn ruler_set_property(
    axes: &mut Axes,
    axis: usize,
    name: &str,
    value: &Value,
    cancel: &AtomicBool,
) -> Result<()> {
    match name {
        "tickvalues" => {
            let ticks = super::plot_time_axes::parse_ruler_values(
                &axes.extras.rulers[axis].kind,
                value,
                "TickValues",
                cancel,
            )?;
            crate::plot::validate_ticks(&ticks)?;
            axes.extras.rulers[axis].ticks = Some(ticks);
        }
        "tickvaluesmode" => match super::graphics_handles::text(value)?
            .to_ascii_lowercase()
            .as_str()
        {
            "auto" => axes.extras.rulers[axis].ticks = None,
            "manual" => {
                let (ticks, _) = axes.effective_ticks(axis);
                axes.extras.rulers[axis].ticks = Some(ticks);
            }
            _ => return Err("TickValuesMode must be auto or manual".into()),
        },
        "ticklabels" => {
            let labels = parse_tick_labels(value, cancel)?;
            axes.extras.rulers[axis].tick_labels = Some(labels);
        }
        "ticklabelsmode" => match super::graphics_handles::text(value)?
            .to_ascii_lowercase()
            .as_str()
        {
            "auto" => axes.extras.rulers[axis].tick_labels = None,
            "manual" => {
                let labels = axes.effective_tick_labels(axis);
                axes.extras.rulers[axis].tick_labels = Some(labels);
            }
            _ => return Err("TickLabelsMode must be auto or manual".into()),
        },
        "ticklabelrotation" => {
            let deg = value.clone().matrix(cancel)?.as_scalar()?;
            if !deg.is_finite() {
                return Err("TickLabelRotation must be finite".into());
            }
            axes.extras.rulers[axis].tick_label_rotation = deg;
        }
        "limits" => {
            let vals = super::plot_time_axes::parse_ruler_values(
                &axes.extras.rulers[axis].kind,
                value,
                "Limits",
                cancel,
            )?;
            if vals.len() != 2 {
                return Err("Limits must be a two-element vector".into());
            }
            let lim = [vals[0], vals[1]];
            crate::plot::validate_axis_limits(lim)?;
            let stored = if lim == [f64::NEG_INFINITY, f64::INFINITY] {
                None
            } else {
                Some(lim)
            };
            match axis {
                0 => axes.xlim = stored,
                1 => axes.ylim = stored,
                _ => axes.zlim = stored,
            }
        }
        "limitsmode" => match super::graphics_handles::text(value)?
            .to_ascii_lowercase()
            .as_str()
        {
            "auto" => match axis {
                0 => axes.xlim = None,
                1 => axes.ylim = None,
                _ => axes.zlim = None,
            },
            "manual" => {
                let b = match axis {
                    0 => Some(axes.bounds().0),
                    1 => Some(axes.bounds().1),
                    _ => Some(axes.zbounds()),
                };
                match axis {
                    0 => axes.xlim = b,
                    1 => axes.ylim = b,
                    _ => axes.zlim = b,
                }
            }
            _ => return Err("LimitsMode must be auto or manual".into()),
        },
        "exponent" => {
            let exp = value.clone().matrix(cancel)?.as_scalar()?;
            if !exp.is_finite() || exp.fract() != 0. || !(-308. ..=308.).contains(&exp) {
                return Err("Exponent must be an integer in -308..=308".into());
            }
            axes.extras.rulers[axis].exponent = exp as i32;
        }
        "ticklabelformat" => {
            let fmt = super::graphics_handles::text(value)?;
            if fmt.is_empty() || fmt == "%g" || fmt.eq_ignore_ascii_case("auto") {
                axes.extras.rulers[axis].tick_label_format = None;
            } else {
                super::plot_time_axes::validate_tick_format(&axes.extras.rulers[axis].kind, fmt)?;
                axes.extras.rulers[axis].tick_label_format = Some(fmt.to_owned());
            }
        }
        "color" => {
            let rgb = super::graphics_handles::axes_rgb(value, cancel)?;
            let colors = axes.colors.get_or_insert_with(Default::default);
            match axis {
                0 => colors.x = Some(rgb),
                1 => colors.y = Some(rgb),
                _ => colors.z = Some(rgb),
            }
        }
        _ => return Err(format!("Unsupported NumericRuler property '{name}'")),
    }
    Ok(())
}
