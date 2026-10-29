//! Real patch/textbox objects using the ordinary name/value grammar.
//! This child module is composed under graphics_handles, not an export parser.
use super::{
    Engine, GraphicsArray, GraphicsHandle, GraphicsKind, Result, Value, handle, text, vector,
};
use crate::{
    Matrix,
    plot::{Series, TextAlign, TextBox},
};

fn pairs(args: &[Value]) -> Result<&[[Value; 2]]> {
    if args.len() > 32 || !args.len().is_multiple_of(2) {
        return Err("Graphics construction accepts at most 16 property/value pairs".into());
    }
    Ok(args.as_chunks::<2>().0)
}
fn rgb(value: &Value) -> Result<[u8; 3]> {
    let values = vector(value, 3)?;
    if values
        .iter()
        .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
    {
        return Err("RGB components must be finite values in 0..1".into());
    }
    Ok(std::array::from_fn(|i| (values[i] * 255.).round() as u8))
}
fn on_off(value: &Value) -> Result<bool> {
    match text(value)?.to_ascii_lowercase().as_str() {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err("Expected on or off".into()),
    }
}
fn literal_only(value: &Value, expected: &str) -> Result<()> {
    if !text(value)?.eq_ignore_ascii_case(expected) {
        return Err(format!("This property supports only '{expected}'"));
    }
    Ok(())
}
fn edge_color(value: &Value) -> Result<Option<[u8; 3]>> {
    if matches!(value, Value::Text(s) if s.eq_ignore_ascii_case("none")) {
        Ok(None)
    } else {
        Ok(Some(super::super::plot_properties::color(value)?))
    }
}
fn line_pattern(value: &Value) -> Result<crate::plot::LinePattern> {
    use crate::plot::LinePattern;
    match text(value)? {
        "-" => Ok(LinePattern::Solid),
        "--" => Ok(LinePattern::Dashed),
        ":" => Ok(LinePattern::Dotted),
        "-." => Ok(LinePattern::DashDot),
        "none" => Ok(LinePattern::None),
        _ => Err("LineStyle supports -, --, :, -. or none".into()),
    }
}
fn line_width(value: &Value) -> Result<f64> {
    let width = vector(value, 1)?[0];
    crate::plot::validate_line_width(width)?;
    Ok(width)
}
pub(super) fn patch_properties(target: &mut Series, args: &[Value]) -> Result<()> {
    for pair in pairs(args)? {
        match text(&pair[0])?.to_ascii_lowercase().as_str() {
            "facecolor" => {
                target.color = rgb(&pair[1])?;
                target.explicit_color = true;
            }
            "edgecolor" => target.face_edge_color = edge_color(&pair[1])?,
            "linestyle" => target.pattern = line_pattern(&pair[1])?,
            "linewidth" => target.line_width = Some(line_width(&pair[1])?),
            "displayname" => {
                let value = text(&pair[1])?;
                if value.len() > 4096 {
                    return Err("Patch label exceeds 4096 bytes".into());
                }
                target.label = value.into();
            }
            "visible" => target.visible = on_off(&pair[1])?,
            _ => {
                return Err(
                    "Patch set supports constant FaceColor, RGB/none EdgeColor, LineStyle, LineWidth, DisplayName and Visible".into(),
                );
            }
        }
    }
    Ok(())
}
fn textbox_properties(target: &mut TextBox, args: &[Value], constructor: bool) -> Result<()> {
    let mut literal = false;
    let mut no_edge = false;
    for pair in pairs(args)? {
        match text(&pair[0])?.to_ascii_lowercase().as_str() {
            "string" => {
                let value = text(&pair[1])?;
                if value.len() > 4096 {
                    return Err("Textbox text exceeds 4096 bytes".into());
                }
                target.text = value.into();
            }
            "position" => target.position = vector(&pair[1], 4)?.try_into().unwrap(),
            "units" => literal_only(&pair[1], "normalized")?,
            "interpreter" => {
                literal_only(&pair[1], "none")?;
                literal = true;
            }
            "edgecolor" => {
                literal_only(&pair[1], "none")?;
                no_edge = true;
            }
            "backgroundcolor" => literal_only(&pair[1], "none")?,
            "fitboxtotext" => literal_only(&pair[1], "off")?,
            "verticalalignment" => literal_only(&pair[1], "top")?,
            "horizontalalignment" => {
                target.horizontal = match text(&pair[1])?.to_ascii_lowercase().as_str() {
                    "left" => TextAlign::Left,
                    "center" => TextAlign::Center,
                    "right" => TextAlign::Right,
                    _ => return Err("Textbox alignment supports left, center or right".into()),
                }
            }
            "fontweight" => {
                target.bold = match text(&pair[1])?.to_ascii_lowercase().as_str() {
                    "normal" => false,
                    "bold" => true,
                    _ => return Err("Textbox FontWeight supports normal or bold".into()),
                }
            }
            "fontsize" => target.font_size = vector(&pair[1], 1)?[0],
            "color" => target.color = rgb(&pair[1])?,
            "visible" => target.visible = on_off(&pair[1])?,
            _ => return Err("Unsupported textbox property; no graph change was published".into()),
        }
    }
    if constructor && (!literal || !no_edge) {
        return Err("Textbox construction requires explicit Interpreter none and EdgeColor none; MATLAB TeX/default outline are unsupported".into());
    }
    target.validate()
}

impl Engine {
    pub(super) fn create_patch_object(&mut self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let (mut target, args) =
            if let Some(value @ Value::Container(crate::Array::Graphics(_))) = args.first() {
                let h = handle(value)?;
                if h.kind() != GraphicsKind::Axes {
                    return Err("Leading patch target must be an axes handle".into());
                }
                (Some(self.locate_graphics(h)?), &args[1..])
            } else {
                (None, args)
            };
        let mut x = None;
        let mut y = None;
        let mut color = None;
        let mut edge = None;
        let mut pattern = None;
        let mut width = None;
        let mut label = String::new();
        let mut visible = true;
        for pair in pairs(args)? {
            match text(&pair[0])?.to_ascii_lowercase().as_str() {
                "parent"=>{
                    if target.is_some() {return Err("Specify one patch axes target, not both a leading target and Parent".into());}
                    let h=handle(&pair[1])?;if h.kind()!=GraphicsKind::Axes {return Err("Patch Parent must be a live axes".into());}
                    target=Some(self.locate_graphics(h)?);
                },
                "xdata"=>{let Value::Matrix(a)=&pair[1] else {return Err("Patch XData requires a real numeric matrix".into());};x=Some(a);},
                "ydata"=>{let Value::Matrix(a)=&pair[1] else {return Err("Patch YData requires a real numeric matrix".into());};y=Some(a);},
                "facecolor"=>color=Some(rgb(&pair[1])?),
                "edgecolor"=>edge=Some(edge_color(&pair[1])?),
                "linestyle"=>pattern=Some(line_pattern(&pair[1])?),
                "linewidth"=>width=Some(line_width(&pair[1])?),
                "displayname"=>{let value=text(&pair[1])?;if value.len()>4096 {return Err("Patch label exceeds 4096 bytes".into());}label=value.into();},
                "visible"=>visible=on_off(&pair[1])?,
                _=>return Err("Unsupported patch property; named XData/YData with constant FaceColor and RGB/none EdgeColor are supported".into()),
            }
        }
        let edge = edge.ok_or("Patch construction requires explicit RGB/none EdgeColor")?;
        let color = color.ok_or("Patch construction requires explicit RGB FaceColor")?;
        let x = x.ok_or("Patch requires XData")?;
        let y = y.ok_or("Patch requires YData")?;
        let id = target.map_or(self.current_figure, |t| t.figure);
        let mut figure = self
            .figures()
            .find(|(n, _)| *n == id)
            .map(|(_, f)| f.clone())
            .unwrap_or_default();
        figure.validate()?;
        let previous = figure.axes.identity.clone();
        if let Some(target) = target {
            super::select_live_axes(&mut figure, target.position.unwrap())?;
        }
        // Retained/expanded resources are checked before any dense pair copy.
        let mut series = Series::patch_from_matrices_with_ledger(
            x,
            y,
            color,
            &self.cancel,
            figure.geometry_ledger()?,
        )?;
        series.face_edge_color = edge;
        series.pattern = pattern.unwrap_or(if edge.is_some() {
            crate::plot::LinePattern::Solid
        } else {
            crate::plot::LinePattern::None
        });
        series.line_width = width;
        series.label = label.clone();
        series.visible = visible;
        let token =
            GraphicsHandle::series(&figure.identity, &figure.axes.identity, &series.identity);
        figure.add(vec![series])?;
        // Low-level patch's empty DisplayName is meaningful; the generic model
        // add helper assigns display defaults for ordinary high-level plots.
        figure.axes.series.last_mut().unwrap().label = label;
        let previous_position = figure
            .all_axes()
            .find(|(_, a)| a.identity.same_object(&previous))
            .map(|(p, _)| p);
        if let Some(position) = previous_position {
            super::select_live_axes(&mut figure, position)?;
        }
        figure.prepare_legend_layout()?;
        figure.validate()?;
        let result = GraphicsArray::scalar(token, &self.cancel)?;
        #[cfg(test)]
        super::tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(id, figure);
        Ok(Value::Container(result.into()))
    }

    pub(super) fn create_textbox_object(&mut self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let (id, args) =
            if let Some(value @ Value::Container(crate::Array::Graphics(_))) = args.first() {
                let h = handle(value)?;
                if h.kind() != GraphicsKind::Figure {
                    return Err("Annotation parent must be a figure handle".into());
                }
                (self.locate_graphics(h)?.figure, &args[1..])
            } else {
                (self.current_figure, args)
            };
        if args.len() < 2 || !text(&args[0])?.eq_ignore_ascii_case("textbox") {
            return Err("Use annotation([figure,] 'textbox', position, properties)".into());
        }
        let mut textbox = TextBox::new(vector(&args[1], 4)?.try_into().unwrap(), String::new())?;
        textbox_properties(&mut textbox, &args[2..], true)?;
        let mut figure = self
            .figures()
            .find(|(n, _)| *n == id)
            .map(|(_, f)| f.clone())
            .unwrap_or_default();
        let token = GraphicsHandle::textbox(&figure.identity, &textbox.identity);
        figure.add_textbox(textbox)?;
        figure.validate()?;
        let result = GraphicsArray::scalar(token, &self.cancel)?;
        #[cfg(test)]
        super::tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(id, figure);
        Ok(Value::Container(result.into()))
    }

    pub(super) fn get_textbox_property(
        &self,
        id: u32,
        index: usize,
        args: &[Value],
    ) -> Result<Value> {
        if args.len() != 1 {
            return Err("get requires one explicit textbox property".into());
        }
        let figure = self.graphics_figure(id)?;
        let target = figure
            .textboxes
            .get(index)
            .ok_or("Textbox no longer exists")?;
        let string = |s: &str| Ok(Value::Text(s.into()));
        match text(&args[0])?.to_ascii_lowercase().as_str() {
            "string" => string(&target.text),
            "parent" => Ok(Value::Container(
                GraphicsArray::scalar(GraphicsHandle::figure(&figure.identity), &self.cancel)?
                    .into(),
            )),
            "position" => Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                4,
                &target.position,
            )?)),
            "fontsize" => Ok(Value::Matrix(Matrix::scalar(target.font_size))),
            "fontweight" => string(if target.bold { "bold" } else { "normal" }),
            "horizontalalignment" => string(match target.horizontal {
                TextAlign::Left => "left",
                TextAlign::Center => "center",
                TextAlign::Right => "right",
            }),
            "verticalalignment" => string("top"),
            "units" => string("normalized"),
            "interpreter" | "edgecolor" | "backgroundcolor" => string("none"),
            "fitboxtotext" => string("off"),
            "visible" => string(if target.visible { "on" } else { "off" }),
            "color" => Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                3,
                &target.color.map(|c| f64::from(c) / 255.),
            )?)),
            _ => Err("Unsupported textbox property query".into()),
        }
    }
    pub(super) fn set_textbox_properties(
        &mut self,
        id: u32,
        index: usize,
        args: &[Value],
    ) -> Result<()> {
        if args.is_empty() {
            return Err("set requires textbox property/value pairs".into());
        }
        let mut figure = self.graphics_figure(id)?.clone();
        textbox_properties(
            figure
                .textboxes
                .get_mut(index)
                .ok_or("Textbox no longer exists")?,
            args,
            false,
        )?;
        figure.validate()?;
        #[cfg(test)]
        super::tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(id, figure);
        Ok(())
    }
}
