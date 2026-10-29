//! Ordinary legend handles and explicit series subsets; no exporter detection.
use super::{Engine, GraphicsArray, GraphicsHandle, GraphicsKind, Located, Result, Value, text};
use crate::{
    Array, CellArray, CharacterRow, ObjectIdentity,
    plot::{Axes, Figure, LegendLocation, LegendState},
};
use std::sync::Arc;

fn property(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "location"
            | "visible"
            | "autoupdate"
            | "interpreter"
            | "string"
            | "box"
            | "color"
            | "textcolor"
            | "edgecolor"
    )
}
fn on_off(value: &Value) -> Result<bool> {
    match text(value)?.to_ascii_lowercase().as_str() {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err("Legend option must be on or off".into()),
    }
}

impl Engine {
    fn legend_labels(&self, value: &Value) -> Result<Vec<String>> {
        let values = match value {
            Value::Container(Array::Cell(cell)) => {
                if cell.len() > crate::plot::MAX_SERIES
                    || (!cell.is_empty() && cell.rows() != 1 && cell.cols() != 1)
                {
                    return Err(
                        "Legend labels require a cell vector of at most 64 character rows".into(),
                    );
                }
                let mut labels = Vec::new();
                for i in 0..cell.len() {
                    self.check_cancel()?;
                    let Array::Character(row) = cell.get_linear(i)?.as_ref() else {
                        return Err("Each legend label must be a character row".into());
                    };
                    let label = row.to_utf8(&self.cancel)?;
                    if label.len() > 4096 {
                        return Err("Legend label exceeds 4096 UTF-8 bytes".into());
                    }
                    labels.push(label);
                }
                labels
            }
            Value::Text(label) => vec![label.clone()],
            _ => {
                return Err(
                    "Legend labels require character text or a cell vector of character rows"
                        .into(),
                );
            }
        };
        if values.iter().any(|s| s.len() > 4096) {
            return Err("Legend label exceeds 4096 UTF-8 bytes".into());
        }
        Ok(values)
    }
    fn apply_legend_labels(&self, axes: &mut Axes, labels: Vec<String>) -> Result<()> {
        let indices: Vec<_> = axes.legend_members().map(|(i, _)| i).collect();
        if labels.len() != indices.len() {
            return Err("Provide exactly one label per selected legend series".into());
        }
        for (index, label) in indices.into_iter().zip(labels) {
            self.check_cancel()?;
            axes.series[index].label = label;
        }
        Ok(())
    }
    fn apply_legend_properties(&self, axes: &mut Axes, args: &[Value]) -> Result<()> {
        if args.len() > 32 || !args.len().is_multiple_of(2) {
            return Err("Legend accepts at most 16 property/value pairs".into());
        }
        for pair in args.as_chunks::<2>().0 {
            self.check_cancel()?;
            match text(&pair[0])?.to_ascii_lowercase().as_str() {
                "location"=>{
                    let location=LegendLocation::parse(text(&pair[1])?)?;
                    if location!=axes.legend_location && let Some(placement)=&mut axes.placement {placement.imported_solved=false;}
                    axes.legend_location=location;
                },
                "color" | "textcolor" | "edgecolor" => {
                    let rgb = super::axes_rgb(&pair[1], &self.cancel)?;
                    let style = axes.colors.get_or_insert_with(Default::default).legend.get_or_insert_with(Default::default);
                    match text(&pair[0])?.to_ascii_lowercase().as_str() {
                        "color" => style.background = Some(rgb),
                        "textcolor" => style.text = Some(rgb),
                        _ => style.border = Some(rgb),
                    }
                },
                "visible"=>axes.legend=on_off(&pair[1])?,
                "autoupdate"=>axes.legend_state.as_mut().ok_or("Legend object is absent")?.auto_update=on_off(&pair[1])?,
                "string"=>self.apply_legend_labels(axes,self.legend_labels(&pair[1])?)?,
                "interpreter" if text(&pair[1])?.eq_ignore_ascii_case("none")=>{},
                "box" if text(&pair[1])?.eq_ignore_ascii_case("on")=>{},
                _=>return Err("Legend supports Location, Visible, AutoUpdate, String, Color, TextColor, EdgeColor, Interpreter none and Box on; no property change was published".into()),
            }
        }
        Ok(())
    }
    fn finish_legend_update(
        &mut self,
        id: u32,
        mut figure: Figure,
        previous: &ObjectIdentity,
    ) -> Result<()> {
        // The edited legend belongs to the currently targeted axes. Initialize
        // its held allocation before restoring the caller's active axes.
        figure.prepare_legend_layout()?;
        let position = figure
            .all_axes()
            .find(|(_, a)| a.identity.same_object(previous))
            .map(|(p, _)| p)
            .ok_or("Legend edit removed the active axes")?;
        super::select_live_axes(&mut figure, position)?;
        figure.validate()?;
        #[cfg(test)]
        super::tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(id, figure);
        Ok(())
    }
    pub(super) fn legend_object_call(&mut self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        if args.len() > 98 {
            return Err("Legend call exceeds bounded labels and properties".into());
        }
        let mut start = 0;
        let mut target = None;
        if let Some(Value::Container(Array::Graphics(handles))) = args.first()
            && handles.len() == 1
            && handles.scalar_handle()?.kind() == GraphicsKind::Axes
        {
            target = Some(self.locate_graphics(handles.scalar_handle()?)?);
            start = 1;
        }
        let mut subset = None;
        if let Some(Value::Container(Array::Graphics(handles))) = args.get(start) {
            if handles.len() > crate::plot::MAX_SERIES
                || (!handles.is_empty() && handles.rows() != 1 && handles.cols() != 1)
            {
                return Err("Legend subset requires a vector of at most 64 series handles".into());
            }
            let mut members = Vec::new();
            for i in 0..handles.len() {
                self.check_cancel()?;
                let h = handles.get_linear(i)?;
                if h.kind() != GraphicsKind::Series {
                    return Err("Legend subset accepts only series handles".into());
                }
                let located = self.locate_graphics(h)?;
                if target.is_some_and(|t: Located| {
                    t.figure != located.figure || t.position != located.position
                }) {
                    return Err("Legend members must belong to one target axes".into());
                }
                if members
                    .iter()
                    .any(|old: &ObjectIdentity| old.same_object(&h.object))
                {
                    return Err("Legend subset contains a duplicate handle".into());
                }
                target = Some(located);
                members.push(h.object.clone());
            }
            subset = Some(members);
            start += 1;
        } else if let Some(Value::Matrix(empty)) = args.get(start)
            && empty.is_empty()
        {
            subset = Some(Vec::new());
            start += 1;
        }
        let id = target.map_or(self.current_figure, |t| t.figure);
        if !self.figures().any(|(n, _)| n == id) && self.figure_ids().len() >= 16 {
            return Err("Legend would exceed 16 figures".into());
        }
        let mut figure = self
            .figures()
            .find(|(n, _)| *n == id)
            .map(|(_, f)| f.clone())
            .unwrap_or_default();
        figure.validate()?;
        figure.activate_tile();
        let previous = figure.axes.identity.clone();
        if let Some(target) = target {
            super::select_live_axes(&mut figure, target.position.unwrap())?;
        }
        let tail = &args[start..];
        let mut labels_end = tail.len();
        while labels_end >= 2 && matches!(&tail[labels_end-2],Value::Text(name) if property(name)) {
            labels_end -= 2;
        }
        let command = if subset.is_none() && labels_end == 1 {
            match &tail[0] {
                Value::Text(s)
                    if matches!(
                        s.to_ascii_lowercase().as_str(),
                        "off" | "hide" | "show" | "on" | "toggle"
                    ) =>
                {
                    Some(s.to_ascii_lowercase())
                }
                _ => None,
            }
        } else {
            None
        };
        if command.as_deref() == Some("off") {
            if labels_end != tail.len() {
                return Err("legend off does not accept properties".into());
            }
            figure.axes.legend = false;
            figure.axes.legend_state = None;
            if let Some(colors) = &mut figure.axes.colors {
                colors.legend = None;
            }
            let result = GraphicsArray::from_shared(0, 0, Vec::new(), &self.cancel)?;
            self.finish_legend_update(id, figure, &previous)?;
            return Ok(Value::Container(result.into()));
        }
        if figure.axes.legend_state.is_none() {
            figure.axes.legend_state = Some(LegendState {
                identity: ObjectIdentity::default(),
                members: figure
                    .axes
                    .series
                    .iter()
                    .map(|s| s.identity.clone())
                    .collect(),
                auto_update: true,
            });
        }
        if let Some(members) = subset {
            figure.axes.legend_state.as_mut().unwrap().members = members;
        }
        if command.is_none() && labels_end != 0 {
            let labels = if labels_end == 1 {
                self.legend_labels(&tail[0])?
            } else {
                if labels_end > crate::plot::MAX_SERIES {
                    return Err("Legend exceeds 64 labels".into());
                }
                let mut labels = Vec::new();
                for value in &tail[..labels_end] {
                    self.check_cancel()?;
                    let label = text(value)?;
                    if label.len() > 4096 {
                        return Err("Legend label exceeds 4096 UTF-8 bytes".into());
                    }
                    labels.push(label.to_owned());
                }
                labels
            };
            self.apply_legend_labels(&mut figure.axes, labels)?;
        }
        figure.axes.legend = match command.as_deref() {
            Some("hide") => false,
            Some("toggle") => !figure.axes.legend,
            _ => true,
        };
        self.apply_legend_properties(&mut figure.axes, &tail[labels_end..])?;
        let state = figure.axes.legend_state.as_ref().unwrap();
        let result = GraphicsArray::scalar(
            GraphicsHandle::legend(&figure.identity, &figure.axes.identity, &state.identity),
            &self.cancel,
        )?;
        self.finish_legend_update(id, figure, &previous)?;
        Ok(Value::Container(result.into()))
    }
    pub(super) fn get_legend_property(&self, located: Located, args: &[Value]) -> Result<Value> {
        if args.len() != 1 {
            return Err("get requires one explicit legend property".into());
        }
        let figure = self.graphics_figure(located.figure)?;
        let (_, axes) = figure
            .all_axes()
            .find(|(p, _)| Some(*p) == located.position)
            .ok_or("Legend axes no longer exists")?;
        let state = axes
            .legend_state
            .as_ref()
            .ok_or("Legend no longer exists")?;
        let value=match text(&args[0])?.to_ascii_lowercase().as_str() {
            "color" | "textcolor" | "edgecolor" => {
                let colors = axes.legend_colors().resolved(figure.resolve_appearance(self.graphics_theme_context));
                let index = match text(&args[0])?.to_ascii_lowercase().as_str() {
                    "color" => 0, "textcolor" => 1, _ => 2,
                };
                return Ok(Value::Matrix(crate::Matrix::from_column_slice(1, 3, &colors[index])?));
            },
            "type"=>"legend", "location"=>axes.legend_location.name(),
            "visible"=>if axes.legend {"on"}else{"off"},
            "autoupdate"=>if state.auto_update {"on"}else{"off"},
            "interpreter"=>"none","box"=>"on",
            "string"=>{
                let mut labels=Vec::new();
                for (_,series) in axes.legend_members() {self.check_cancel()?;labels.push(Arc::new(CharacterRow::from_text(&series.label,&self.cancel)?.into()));}
                return Ok(Value::Container(CellArray::from_shared(labels.len(),1,labels,&self.cancel)?.into()));
            },
            _=>return Err("Unsupported legend query; supported properties are Type, Location, Visible, AutoUpdate, String, Color, TextColor, EdgeColor, Interpreter and Box".into()),
        };
        Ok(Value::Text(value.into()))
    }
    pub(super) fn set_legend_properties(&mut self, located: Located, args: &[Value]) -> Result<()> {
        let mut figure = self.graphics_figure(located.figure)?.clone();
        let previous = figure.axes.identity.clone();
        super::select_live_axes(&mut figure, located.position.unwrap())?;
        self.apply_legend_properties(&mut figure.axes, args)?;
        self.finish_legend_update(located.figure, figure, &previous)
    }
}
