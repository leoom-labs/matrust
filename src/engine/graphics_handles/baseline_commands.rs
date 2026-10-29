//! Genuine chart baseline objects; observation and mutation are separate.
use super::*;
use crate::plot::{Axes, LinePattern};

fn visibility(value: &Value) -> Result<bool> {
    match value {
        Value::Text(s) if s.eq_ignore_ascii_case("on") => Ok(true),
        Value::Text(s) if s.eq_ignore_ascii_case("off") => Ok(false),
        Value::Logical(v) => v.as_scalar(),
        Value::Matrix(v) if v.as_scalar()? == 0. => Ok(false),
        Value::Matrix(v) if v.as_scalar()? == 1. => Ok(true),
        _ => Err("Baseline visibility requires on, off, logical or numeric 0/1".into()),
    }
}
fn pattern(value: &Value) -> Result<LinePattern> {
    match text(value)? {
        "-" => Ok(LinePattern::Solid),
        "--" => Ok(LinePattern::Dashed),
        ":" => Ok(LinePattern::Dotted),
        "-." => Ok(LinePattern::DashDot),
        "none" => Ok(LinePattern::None),
        _ => Err("Baseline LineStyle supports -, --, :, -. or none".into()),
    }
}
fn pattern_text(pattern: LinePattern) -> &'static str {
    match pattern {
        LinePattern::Solid => "-",
        LinePattern::Dashed => "--",
        LinePattern::Dotted => ":",
        LinePattern::DashDot => "-.",
        LinePattern::None => "none",
    }
}
fn rgb(value: &Value) -> Result<[f64; 3]> {
    if matches!(value, Value::Matrix(_)) {
        let values: [f64; 3] = vector(value, 3)?.try_into().unwrap();
        if values
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err("Baseline Color requires RGB components in 0..1".into());
        }
        Ok(values)
    } else {
        Ok(super::super::plot_properties::color(value)?.map(|v| f64::from(v) / 255.))
    }
}

impl Engine {
    pub(super) fn get_baseline_property(
        &self,
        h: &GraphicsHandle,
        located: Located,
        args: &[Value],
    ) -> Result<Value> {
        self.check_cancel()?;
        let figure = self.graphics_figure(located.figure)?;
        let (_, axes) = figure
            .all_axes()
            .find(|(p, _)| Some(*p) == located.position)
            .ok_or("Baseline axes is no longer live")?;
        let observed = axes.observe_baseline(&h.object)?;
        match text(&args[0])?.to_ascii_lowercase().as_str() {
            "type" => Ok(Value::Text("baseline".into())),
            "parent" => Ok(Value::Container(GraphicsArray::scalar(
                GraphicsHandle::axes(&figure.identity, &axes.identity), &self.cancel)?.into())),
            "basevalue" => Ok(Value::Matrix(Matrix::scalar(axes.baseline_value_cancellable(&h.object, &self.cancel)?))),
            "visible" => Ok(Value::Text(if observed.style.visible {"on"} else {"off"}.into())),
            "color" => Ok(Value::Matrix(Matrix::from_column_slice(1,3,&observed.style.color)?)),
            "linewidth" => Ok(Value::Matrix(Matrix::scalar(observed.style.width_points))),
            "linestyle" => Ok(Value::Text(pattern_text(observed.style.pattern).into())),
            _ => Err("Baseline queries support Type, Parent, BaseValue, Visible, Color, LineWidth and LineStyle".into()),
        }
    }

    pub(super) fn set_baseline_properties(
        &mut self,
        h: &GraphicsHandle,
        located: Located,
        args: &[Value],
    ) -> Result<()> {
        self.check_cancel()?;
        let mut figure = self.graphics_figure(located.figure)?.clone();
        let previous = figure.axes.identity.clone();
        select_live_axes(
            &mut figure,
            located.position.ok_or("Baseline axes is missing")?,
        )?;
        let mut style = figure.axes.observe_baseline(&h.object)?.style;
        let mut value = None;
        let mut style_changed = false;
        for pair in args.as_chunks::<2>().0 {
            self.check_cancel()?;
            match text(&pair[0])?.to_ascii_lowercase().as_str() {
                "basevalue" => value = Some(vector(&pair[1], 1)?[0]),
                "visible" => {
                    style.visible = visibility(&pair[1])?;
                    style_changed = true;
                }
                "color" => {
                    style.color = rgb(&pair[1])?;
                    style_changed = true;
                }
                "linestyle" => {
                    style.pattern = pattern(&pair[1])?;
                    style_changed = true;
                }
                "linewidth" => {
                    style.width_points = vector(&pair[1], 1)?[0];
                    style_changed = true;
                }
                _ => {
                    return Err(
                        "Baseline set supports BaseValue, Visible, Color, LineWidth and LineStyle"
                            .into(),
                    );
                }
            }
        }
        figure.axes.set_observed_baseline(
            &h.object,
            value,
            style_changed.then_some(style),
            &self.cancel,
        )?;
        let old_position = figure
            .all_axes()
            .find(|(_, a)| a.identity.same_object(&previous))
            .map(|(p, _)| p)
            .ok_or("Previously active axes is no longer live")?;
        select_live_axes(&mut figure, old_position)?;
        figure.validate()?;
        #[cfg(test)]
        super::tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(located.figure, figure);
        Ok(())
    }

    /// Called inside the ordinary series set transaction; unrelated properties
    /// still use their existing parser and the outer figure is published once.
    pub(super) fn set_chart_properties(
        &self,
        axes: &mut Axes,
        index: usize,
        args: &[Value],
    ) -> Result<()> {
        let kind = axes.series[index].kind;
        let baseline = axes.observe_series_baseline(index)?;
        let identity = baseline.identity.clone();
        let mut style = baseline.style;
        let mut value = None;
        let mut style_changed = false;
        let mut ordinary = Vec::with_capacity(args.len());
        for pair in args.as_chunks::<2>().0 {
            match text(&pair[0])?.to_ascii_lowercase().as_str() {
                "basevalue" => value=Some(vector(&pair[1],1)?[0]),
                "showbaseline" => {style.visible=visibility(&pair[1])?;style_changed=true;}
                "baseline" => return Err("Chart BaseLine is read-only; edit its properties through get(chart,'BaseLine')".into()),
                _ => ordinary.extend(pair.iter().cloned()),
            }
        }
        if !ordinary.is_empty() {
            let (start, properties) =
                super::super::plot_properties::Properties::parse(&ordinary, kind)?;
            if start != 0 {
                return Err("set requires property/value pairs only".into());
            }
            properties.apply(&mut axes.series[index..index + 1], kind)?;
        }
        if value.is_some()
            && !style_changed
            && kind == crate::plot::PlotKind::Stem
            && axes
                .baseline_for_series(&axes.series[index].identity)
                .is_none()
        {
            // Existing BaseValue-only chart edits already have a lossless field.
            // Keep their baseline-free snapshots and script behavior unchanged.
            axes.series[index].base_value = value;
            axes.series[index].validate_base_value()?;
        } else if value.is_some() || style_changed {
            axes.set_observed_baseline(
                &identity,
                value,
                style_changed.then_some(style),
                &self.cancel,
            )?;
        }
        Ok(())
    }
}
