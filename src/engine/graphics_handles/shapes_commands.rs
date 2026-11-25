//! Builtin commands and property accessors for `xline`, `yline` and `rectangle`.
use super::{
    Engine, GraphicsArray, GraphicsHandle, GraphicsKind, Result, Value, handle, text, vector,
};
use crate::Matrix;
use crate::plot::{
    ConstantLineHAlign, ConstantLineObject, ConstantLineVAlign, InterceptAxis, LinePattern,
    MAX_CONSTANT_LINES, MAX_RECTANGLES, MAX_TEXT_BYTES, RectangleObject,
};
use std::sync::Arc;

fn is_constant_line_property(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "color"
            | "linestyle"
            | "linewidth"
            | "alpha"
            | "label"
            | "labelhorizontalalignment"
            | "labelverticalalignment"
            | "displayname"
            | "visible"
            | "parent"
            | "value"
            | "interceptaxis"
    )
}

fn parse_linespec(spec: &str) -> Result<(Option<[f64; 3]>, Option<LinePattern>)> {
    if spec.is_empty() {
        return Ok((None, None));
    }
    let mut s = spec;
    let mut pattern = None;
    let mut color = None;
    while !s.is_empty() {
        if let Some(rest) = s.strip_prefix("--") {
            if pattern.is_some() {
                return Err("LineSpec specifies multiple line styles".into());
            }
            pattern = Some(LinePattern::Dashed);
            s = rest;
        } else if let Some(rest) = s.strip_prefix("-.") {
            if pattern.is_some() {
                return Err("LineSpec specifies multiple line styles".into());
            }
            pattern = Some(LinePattern::DashDot);
            s = rest;
        } else if let Some(rest) = s.strip_prefix('-') {
            if pattern.is_some() {
                return Err("LineSpec specifies multiple line styles".into());
            }
            pattern = Some(LinePattern::Solid);
            s = rest;
        } else if let Some(rest) = s.strip_prefix(':') {
            if pattern.is_some() {
                return Err("LineSpec specifies multiple line styles".into());
            }
            pattern = Some(LinePattern::Dotted);
            s = rest;
        } else {
            let mut chars = s.chars();
            let ch = chars.next().unwrap();
            let rgb = match ch {
                'r' => [1., 0., 0.],
                'g' => [0., 1., 0.],
                'b' => [0., 0., 1.],
                'c' => [0., 1., 1.],
                'm' => [1., 0., 1.],
                'y' => [1., 1., 0.],
                'k' => [0., 0., 0.],
                'w' => [1., 1., 1.],
                _ => return Err(format!("Unsupported LineSpec '{spec}'")),
            };
            if color.is_some() {
                return Err("LineSpec specifies multiple colors".into());
            }
            color = Some(rgb);
            s = chars.as_str();
        }
    }
    Ok((color, pattern))
}

fn parse_color_f64(value: &Value) -> Result<[f64; 3]> {
    if let Value::Matrix(m) = value {
        let vals = vector(value, 3)?;
        if vals
            .iter()
            .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
        {
            return Err("RGB components must be finite values in 0..1".into());
        }
        let _ = m;
        return Ok([vals[0], vals[1], vals[2]]);
    }
    let rgb = super::super::plot_properties::color(value)?;
    Ok(rgb.map(|c| f64::from(c) / 255.))
}

fn parse_optional_color_f64(value: &Value) -> Result<Option<[f64; 3]>> {
    if matches!(value, Value::Text(s) if s.eq_ignore_ascii_case("none")) {
        Ok(None)
    } else {
        Ok(Some(parse_color_f64(value)?))
    }
}

fn parse_pattern(value: &Value) -> Result<LinePattern> {
    match text(value)? {
        "-" => Ok(LinePattern::Solid),
        "--" => Ok(LinePattern::Dashed),
        ":" => Ok(LinePattern::Dotted),
        "-." => Ok(LinePattern::DashDot),
        "none" => Ok(LinePattern::None),
        _ => Err("LineStyle supports -, --, :, -. or none".into()),
    }
}

fn pattern_str(pattern: LinePattern) -> &'static str {
    match pattern {
        LinePattern::Solid => "-",
        LinePattern::Dashed => "--",
        LinePattern::Dotted => ":",
        LinePattern::DashDot => "-.",
        LinePattern::None => "none",
    }
}

fn parse_on_off(value: &Value) -> Result<bool> {
    match value {
        Value::Text(s) if s.eq_ignore_ascii_case("on") => Ok(true),
        Value::Text(s) if s.eq_ignore_ascii_case("off") => Ok(false),
        Value::Logical(m) => m.as_scalar(),
        Value::Matrix(m) if m.as_scalar()? == 1. => Ok(true),
        Value::Matrix(m) if m.as_scalar()? == 0. => Ok(false),
        _ => Err("Expected 'on' or 'off'".into()),
    }
}

fn rgb_value(rgb: [f64; 3]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_slice(1, 3, &rgb)?))
}

pub(super) fn constant_line_get(
    line: &ConstantLineObject,
    name: &str,
    parent: Value,
) -> Result<Value> {
    let string = |s: &str| Ok(Value::Text(s.into()));
    match name {
        "type" => string("constantline"),
        "parent" => Ok(parent),
        "value" => Ok(Value::Matrix(Matrix::scalar(line.value))),
        "interceptaxis" => string(line.axis.as_str()),
        "color" => rgb_value(line.color.unwrap_or([0.15, 0.15, 0.15])),
        "linestyle" => string(pattern_str(line.pattern)),
        "linewidth" => Ok(Value::Matrix(Matrix::scalar(line.line_width))),
        "alpha" => Ok(Value::Matrix(Matrix::scalar(line.alpha))),
        "label" => string(&line.label),
        "labelhorizontalalignment" => string(line.label_h_align.as_str()),
        "labelverticalalignment" => string(line.label_v_align.as_str()),
        "displayname" => string(&line.display_name),
        "visible" => string(if line.visible { "on" } else { "off" }),
        _ => Err("Unsupported ConstantLine property query".into()),
    }
}

pub(super) fn constant_line_set(line: &mut ConstantLineObject, pair: &[Value; 2]) -> Result<()> {
    match text(&pair[0])?.to_ascii_lowercase().as_str() {
        "value" => {
            let v = vector(&pair[1], 1)?[0];
            if !v.is_finite() {
                return Err("ConstantLine Value must be finite".into());
            }
            line.value = v;
        }
        "interceptaxis" => {
            line.axis = match text(&pair[1])?.to_ascii_lowercase().as_str() {
                "x" => InterceptAxis::X,
                "y" => InterceptAxis::Y,
                _ => return Err("InterceptAxis must be 'x' or 'y'".into()),
            };
        }
        "color" => line.color = Some(parse_color_f64(&pair[1])?),
        "linestyle" => line.pattern = parse_pattern(&pair[1])?,
        "linewidth" => {
            let w = vector(&pair[1], 1)?[0];
            crate::plot::validate_line_width(w)?;
            line.line_width = w;
        }
        "alpha" => {
            let a = vector(&pair[1], 1)?[0];
            if !a.is_finite() || !(0. ..=1.).contains(&a) {
                return Err("ConstantLine Alpha must be a finite scalar in 0..1".into());
            }
            line.alpha = a;
        }
        "label" => {
            let s = text(&pair[1])?;
            if s.len() > MAX_TEXT_BYTES {
                return Err("ConstantLine label exceeds 4096 UTF-8 bytes".into());
            }
            line.label = s.to_owned();
        }
        "labelhorizontalalignment" => {
            line.label_h_align = ConstantLineHAlign::parse(text(&pair[1])?)?
        }
        "labelverticalalignment" => {
            line.label_v_align = ConstantLineVAlign::parse(text(&pair[1])?)?
        }
        "displayname" => {
            let s = text(&pair[1])?;
            if s.len() > MAX_TEXT_BYTES {
                return Err("ConstantLine DisplayName exceeds 4096 UTF-8 bytes".into());
            }
            line.display_name = s.to_owned();
        }
        "visible" => line.visible = parse_on_off(&pair[1])?,
        other => return Err(format!("Unsupported ConstantLine property '{other}'")),
    }
    line.validate()
}

pub(super) fn rectangle_get(rect: &RectangleObject, name: &str, parent: Value) -> Result<Value> {
    let string = |s: &str| Ok(Value::Text(s.into()));
    match name {
        "type" => string("rectangle"),
        "parent" => Ok(parent),
        "position" => Ok(Value::Matrix(Matrix::from_column_slice(
            1,
            4,
            &rect.position,
        )?)),
        "curvature" => Ok(Value::Matrix(Matrix::from_column_slice(
            1,
            2,
            &rect.curvature,
        )?)),
        "facecolor" => match rect.face_color {
            Some(rgb) => rgb_value(rgb),
            None => string("none"),
        },
        "edgecolor" => match rect.edge_color {
            Some(rgb) => rgb_value(rgb),
            None => string("none"),
        },
        "linewidth" => Ok(Value::Matrix(Matrix::scalar(rect.line_width))),
        "linestyle" => string(pattern_str(rect.pattern)),
        "visible" => string(if rect.visible { "on" } else { "off" }),
        _ => Err("Unsupported Rectangle property query".into()),
    }
}

pub(super) fn rectangle_set(rect: &mut RectangleObject, pair: &[Value; 2]) -> Result<()> {
    match text(&pair[0])?.to_ascii_lowercase().as_str() {
        "position" => {
            let pos = vector(&pair[1], 4)?;
            rect.position = [pos[0], pos[1], pos[2], pos[3]];
        }
        "curvature" => {
            let Value::Matrix(m) = &pair[1] else {
                return Err("Rectangle Curvature requires a scalar or 2-element vector".into());
            };
            let vals = m.data();
            if vals.len() == 1 {
                rect.curvature = [vals[0], vals[0]];
            } else if vals.len() == 2 && (m.rows() == 1 || m.cols() == 1) {
                rect.curvature = [vals[0], vals[1]];
            } else {
                return Err("Rectangle Curvature requires a scalar or 2-element vector".into());
            }
        }
        "facecolor" => rect.face_color = parse_optional_color_f64(&pair[1])?,
        "edgecolor" => rect.edge_color = parse_optional_color_f64(&pair[1])?,
        "linewidth" => {
            let w = vector(&pair[1], 1)?[0];
            crate::plot::validate_line_width(w)?;
            rect.line_width = w;
        }
        "linestyle" => rect.pattern = parse_pattern(&pair[1])?,
        "visible" => rect.visible = parse_on_off(&pair[1])?,
        other => return Err(format!("Unsupported Rectangle property '{other}'")),
    }
    rect.validate()
}

impl Engine {
    fn resolve_axes_target<'a>(
        &self,
        args: &'a [Value],
    ) -> Result<(Option<super::Located>, &'a [Value])> {
        if let Some(value @ Value::Container(crate::Array::Graphics(_))) = args.first() {
            let h = handle(value)?;
            if h.kind() != GraphicsKind::Axes {
                return Err("Leading target handle must be an axes handle".into());
            }
            Ok((Some(self.locate_graphics(h)?), &args[1..]))
        } else {
            Ok((None, args))
        }
    }

    pub(super) fn create_constant_line_object(
        &mut self,
        axis: InterceptAxis,
        args: &[Value],
    ) -> Result<Value> {
        self.check_cancel()?;
        let (mut target, args) = self.resolve_axes_target(args)?;
        if args.is_empty() {
            return Err("xline/yline requires a coordinate value or vector".into());
        }
        let values = match &args[0] {
            Value::Matrix(m) if m.is_empty() || m.rows() == 1 || m.cols() == 1 => m.data().to_vec(),
            _ => return Err("xline/yline value must be a real scalar or vector".into()),
        };
        if values.iter().any(|v| !v.is_finite()) {
            return Err("xline/yline values must be finite".into());
        }
        let (spec_color, spec_pattern, label_arg, pair_args) = if args.len() == 1 {
            (None, None, None, &args[1..])
        } else if let Value::Text(s) = &args[1] {
            if (args.len() - 1).is_multiple_of(2) && is_constant_line_property(s) {
                (None, None, None, &args[1..])
            } else {
                let (c, p) = parse_linespec(s)?;
                if args.len() > 2 && !(args.len() - 2).is_multiple_of(2) {
                    let lbl = text(&args[2])?.to_owned();
                    (c, p, Some(lbl), &args[3..])
                } else {
                    (c, p, None, &args[2..])
                }
            }
        } else {
            return Err("Expected LineSpec or property name".into());
        };
        if pair_args.len() > 32 || !pair_args.len().is_multiple_of(2) {
            return Err("Graphics construction accepts at most 16 property/value pairs".into());
        }
        let mut template = ConstantLineObject::new(axis, 0.);
        if let Some(c) = spec_color {
            template.color = Some(c);
        }
        if let Some(p) = spec_pattern {
            template.pattern = p;
        }
        if let Some(lbl) = label_arg {
            template.label = lbl;
        }
        for pair in pair_args.as_chunks::<2>().0 {
            if text(&pair[0])?.eq_ignore_ascii_case("parent") {
                if target.is_some() {
                    return Err(
                        "Specify one axes target, not both a leading handle and Parent".into(),
                    );
                }
                let h = handle(&pair[1])?;
                if h.kind() != GraphicsKind::Axes {
                    return Err("Parent must be a live axes".into());
                }
                target = Some(self.locate_graphics(h)?);
            } else {
                constant_line_set(&mut template, pair)?;
            }
        }

        let id = target.map_or(self.current_figure, |t| t.figure);
        // graphics_figure also resolves uiaxes backing figures.
        let existing = self.graphics_figure(id).ok().cloned();
        let mut figure = match existing {
            Some(f) => f,
            None => {
                let mut f = crate::plot::Figure::default();
                self.apply_figure_defaults(&mut f)?;
                self.apply_axes_defaults(&mut f)?;
                f
            }
        };
        let previous = figure.axes.identity.clone();
        if let Some(target) = target {
            super::select_live_axes(&mut figure, target.position.unwrap())?;
        }
        if figure.axes.extras.constant_lines.len() + values.len() > MAX_CONSTANT_LINES {
            return Err("Axes exceeds 256 constant line objects".into());
        }
        let mut handles = Vec::with_capacity(values.len());
        for v in values {
            let mut obj = template.clone();
            obj.identity = crate::ObjectIdentity::default();
            obj.value = v;
            obj.validate()?;
            let h = GraphicsHandle::axes_object(
                &figure.identity,
                &figure.axes.identity,
                &obj.identity,
                GraphicsKind::ConstantLine,
            );
            figure.axes.extras.constant_lines.push(obj);
            handles.push(Arc::new(h));
        }
        let previous_position = figure
            .all_axes()
            .find(|(_, a)| a.identity.same_object(&previous))
            .map(|(p, _)| p);
        if let Some(position) = previous_position {
            super::select_live_axes(&mut figure, position)?;
        }
        figure.validate()?;
        let (rows, cols) = if handles.is_empty() {
            (0, 0)
        } else {
            (1, handles.len())
        };
        let array = GraphicsArray::from_shared(rows, cols, handles, &self.cancel)?;
        #[cfg(test)]
        super::tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(id, figure);
        Ok(Value::Container(array.into()))
    }

    pub(super) fn create_rectangle_object(&mut self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let (mut target, args) = self.resolve_axes_target(args)?;
        if args.len() > 32 || !args.len().is_multiple_of(2) {
            return Err("rectangle accepts property/value pairs only".into());
        }
        let mut rect = RectangleObject::new();
        for pair in args.as_chunks::<2>().0 {
            if text(&pair[0])?.eq_ignore_ascii_case("parent") {
                if target.is_some() {
                    return Err(
                        "Specify one axes target, not both a leading handle and Parent".into(),
                    );
                }
                let h = handle(&pair[1])?;
                if h.kind() != GraphicsKind::Axes {
                    return Err("Parent must be a live axes".into());
                }
                target = Some(self.locate_graphics(h)?);
            } else {
                rectangle_set(&mut rect, pair)?;
            }
        }
        rect.validate()?;

        let id = target.map_or(self.current_figure, |t| t.figure);
        // graphics_figure also resolves uiaxes backing figures.
        let existing = self.graphics_figure(id).ok().cloned();
        let mut figure = match existing {
            Some(f) => f,
            None => {
                let mut f = crate::plot::Figure::default();
                self.apply_figure_defaults(&mut f)?;
                self.apply_axes_defaults(&mut f)?;
                f
            }
        };
        let previous = figure.axes.identity.clone();
        if let Some(target) = target {
            super::select_live_axes(&mut figure, target.position.unwrap())?;
        }
        if figure.axes.extras.rectangles.len() >= MAX_RECTANGLES {
            return Err("Axes exceeds 256 rectangle objects".into());
        }
        let token = GraphicsHandle::axes_object(
            &figure.identity,
            &figure.axes.identity,
            &rect.identity,
            GraphicsKind::Rectangle,
        );
        figure.axes.extras.rectangles.push(rect);
        let previous_position = figure
            .all_axes()
            .find(|(_, a)| a.identity.same_object(&previous))
            .map(|(p, _)| p);
        if let Some(position) = previous_position {
            super::select_live_axes(&mut figure, position)?;
        }
        figure.validate()?;
        let result = GraphicsArray::scalar(token, &self.cancel)?;
        #[cfg(test)]
        super::tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(id, figure);
        Ok(Value::Container(result.into()))
    }
}
