//! Genuine typed graphics references. This initial layer does not implement
//! arbitrary figure colors/names or explicit legend objects.
mod baseline_commands;
mod legend_commands;
mod object_commands;
use super::{Engine, Execution, Value};
use crate::plot::{Figure, Position, PositionConstraint};
use crate::{Array, GraphicsArray, GraphicsHandle, GraphicsKind, LogicalMatrix, Matrix, Result};
use std::sync::Arc;

#[derive(Clone, Copy)]
struct Located {
    figure: u32,
    position: Option<Position>,
    series: Option<usize>,
    textbox: Option<usize>,
}
// Handle targeting selects an existing object; it must never manufacture a
// subplot layout merely to select an already-active ordinary axes.
fn select_live_axes(figure: &mut Figure, position: Position) -> Result<()> {
    figure.validate()?;
    if !figure.all_axes().any(|(p, _)| p == position) {
        return Err("Graphics target axes is no longer live".into());
    }
    if figure.active_position() != position {
        figure.select_subplot(position, false)?;
    }
    Ok(())
}

fn handle(value: &Value) -> Result<&GraphicsHandle> {
    match value {
        Value::Container(Array::Graphics(a)) => a.scalar_handle(),
        _ => Err("Expected one typed graphics handle".into()),
    }
}
fn text(value: &Value) -> Result<&str> {
    match value {
        Value::Text(s) => Ok(s),
        _ => Err("Graphics property requires character text".into()),
    }
}
fn vector(value: &Value, n: usize) -> Result<Vec<f64>> {
    let Value::Matrix(a) = value else {
        return Err("Graphics property requires a real vector".into());
    };
    if a.len() != n || (a.rows() != 1 && a.cols() != 1) {
        return Err(format!("Graphics property requires {n} real values"));
    }
    Ok(a.data().to_vec())
}
fn figure_appearance(
    figure: &mut Figure,
    pair: &[Value; 2],
    host: crate::plot::BaseTheme,
) -> Result<()> {
    match text(&pair[0])?.to_ascii_lowercase().as_str() {
        "name" => {
            let name = text(&pair[1])?;
            if name.len() > 4096 {
                return Err("Figure Name exceeds 4096 UTF-8 bytes".into());
            }
            figure.name = name.to_owned();
        }
        "inverthardcopy" => {
            figure.invert_hardcopy = Some(match text(&pair[1])?.to_ascii_lowercase().as_str() {
                "on" => true,
                "off" => false,
                _ => return Err("InvertHardcopy supports on or off".into()),
            });
        }
        "color" => figure.set_canvas_color(super::plot_properties::color(&pair[1])?),
        "theme" => figure.set_figure_theme(match text(&pair[1])?.to_ascii_lowercase().as_str() {
            "light" => crate::plot::FigureTheme::Light,
            "dark" => crate::plot::FigureTheme::Dark,
            _ => return Err("Theme supports light or dark; use ThemeMode auto for host appearance".into()),
        }),
        "thememode" | "colormode" => {
            let mode = match text(&pair[1])?.to_ascii_lowercase().as_str() {
                "auto" => crate::plot::ColorMode::Auto,
                "manual" => crate::plot::ColorMode::Manual,
                _ => return Err("Appearance mode supports auto or manual".into()),
            };
            if text(&pair[0])?.eq_ignore_ascii_case("ThemeMode") { figure.set_theme_mode(mode,host); }
            else { figure.set_canvas_mode(mode,host); }
        }
        _ => return Err(
            "Figure appearance supports Name, Color, Theme, ThemeMode, ColorMode and InvertHardcopy; unsupported properties are not discarded"
                .into(),
        ),
    }
    Ok(())
}
impl Engine {
    pub(super) fn concatenate_graphics(&self, rows: Vec<Vec<Value>>) -> Result<Value> {
        let mut blocks = Vec::new();
        let mut total_rows = 0usize;
        let mut total_cols = None;
        let mut part_count = 0usize;
        for row in rows {
            self.check_cancel()?;
            let mut parts = Vec::new();
            let mut row_height = None;
            let mut columns = 0usize;
            for value in row {
                if matches!(&value,Value::Matrix(m) if m.rows()==0 && m.cols()==0) {
                    continue;
                }
                let Value::Container(Array::Graphics(h)) = value else {
                    return Err(
                        "Graphics concatenation requires handle arrays, with optional numeric []"
                            .into(),
                    );
                };
                part_count += 1;
                if part_count > crate::script_containers::MAX_CELL_SLOTS {
                    return Err("Graphics concatenation exceeds 65536 parts".into());
                }
                if part_count.is_multiple_of(1024) {
                    self.check_cancel()?;
                }
                if row_height.is_some_and(|n| n != h.rows()) {
                    return Err("Graphics concatenation row heights differ".into());
                }
                row_height = Some(h.rows());
                columns = columns
                    .checked_add(h.cols())
                    .ok_or("Graphics concatenation width overflow")?;
                if columns > crate::script_containers::MAX_CELL_SLOTS {
                    return Err("Graphics concatenation exceeds 65536 columns".into());
                }
                parts.push(h);
            }
            let Some(height) = row_height else {
                continue;
            };
            if total_cols.is_some_and(|n| n != columns) {
                return Err("Graphics concatenation row widths differ".into());
            }
            total_cols = Some(columns);
            total_rows = total_rows
                .checked_add(height)
                .ok_or("Graphics concatenation height overflow")?;
            if total_rows > crate::script_containers::MAX_CELL_SLOTS {
                return Err("Graphics concatenation exceeds 65536 rows".into());
            }
            // Zero-height blocks affect width validation but never contribute
            // output. Retaining them would multiply empty traversal work.
            if height != 0 {
                blocks.push((height, parts));
            }
        }
        let columns = total_cols.unwrap_or(0);
        let len = total_rows
            .checked_mul(columns)
            .ok_or("Graphics concatenation size overflow")?;
        if len > crate::script_containers::MAX_CELL_SLOTS {
            return Err("Graphics concatenation exceeds 65536 handles".into());
        }
        let mut values = Vec::new();
        values
            .try_reserve_exact(len)
            .map_err(|_| "Cannot allocate graphics concatenation")?;
        if len == 0 {
            return Ok(Value::Container(
                GraphicsArray::from_shared(total_rows, columns, values, &self.cancel)?.into(),
            ));
        }
        // Every cursor advances monotonically across one row block. Traversal
        // is O(parts + output), including zero-width parts, rather than scanning
        // a 65536-part row again for each output column.
        let mut cursors = Vec::new();
        cursors
            .try_reserve_exact(blocks.len())
            .map_err(|_| "Cannot allocate graphics concatenation cursors")?;
        cursors.resize(blocks.len(), (0usize, 0usize));
        for column in 0..columns {
            if column.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            for ((height, parts), (part_index, offset)) in blocks.iter().zip(&mut cursors) {
                while *part_index < parts.len() && *offset == parts[*part_index].cols() {
                    *part_index += 1;
                    *offset = 0;
                    if (*part_index).is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                }
                let part = parts
                    .get(*part_index)
                    .ok_or("Invalid graphics concatenation column")?;
                for row in 0..*height {
                    if values.len().is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    values.push(part.get_linear(*offset * *height + row)?.clone());
                }
                *offset += 1;
            }
        }
        Ok(Value::Container(
            GraphicsArray::from_shared(total_rows, columns, values, &self.cancel)?.into(),
        ))
    }
    fn locate_graphics(&self, h: &GraphicsHandle) -> Result<Located> {
        let mut found = None;
        // Recheck structural limits because the Rust model fields are public.
        // Never traverse arbitrary externally supplied vectors before bounding
        // them, and never inspect samples to resolve a metadata identity.
        if self.other_figures.len() + usize::from(self.figure.is_some()) > 16 {
            return Err("Graphics lookup exceeds 16 figures".into());
        }
        for (id, f) in self.figures() {
            self.check_cancel()?;
            if f.layout.as_ref().is_some_and(|l| l.others.len() > 15) {
                return Err("Graphics lookup exceeds 16 axes".into());
            }
            if f.textboxes.len() > crate::plot::MAX_TEXTBOXES {
                return Err("Graphics lookup exceeds 64 textboxes".into());
            }
            let mut series_count = 0usize;
            for (_, axes) in f.all_axes() {
                if axes.series.len() > crate::plot::MAX_SERIES {
                    return Err("Graphics lookup exceeds 64 series".into());
                }
                series_count += axes.series.len();
            }
            if series_count > crate::plot::MAX_SERIES {
                return Err("Graphics lookup exceeds 64 total series".into());
            }
            if !f.identity.same_object(&h.figure) {
                continue;
            }
            if h.kind == GraphicsKind::Figure && f.identity.same_object(&h.object) {
                if found.is_some() {
                    return Err("Ambiguous duplicate graphics identity".into());
                }
                found = Some(Located {
                    figure: id,
                    position: None,
                    series: None,
                    textbox: None,
                });
                continue;
            }
            if h.kind == GraphicsKind::TextBox {
                for (index, textbox) in f.textboxes.iter().enumerate() {
                    if h.object.same_object(&textbox.identity) {
                        if found.is_some() {
                            return Err("Ambiguous duplicate graphics identity".into());
                        }
                        found = Some(Located {
                            figure: id,
                            position: None,
                            series: None,
                            textbox: Some(index),
                        });
                    }
                }
                continue;
            }
            for (position, a) in f.all_axes() {
                if !h
                    .axes
                    .as_ref()
                    .is_some_and(|token| token.same_object(&a.identity))
                {
                    continue;
                }
                if h.kind == GraphicsKind::Axes && h.object.same_object(&a.identity) {
                    if found.is_some() {
                        return Err("Ambiguous duplicate graphics identity".into());
                    }
                    found = Some(Located {
                        figure: id,
                        position: Some(position),
                        series: None,
                        textbox: None,
                    });
                }
                if h.kind == GraphicsKind::Legend
                    && a.legend_state
                        .as_ref()
                        .is_some_and(|state| state.identity.same_object(&h.object))
                {
                    if found.is_some() {
                        return Err("Ambiguous duplicate graphics identity".into());
                    }
                    found = Some(Located {
                        figure: id,
                        position: Some(position),
                        series: None,
                        textbox: None,
                    });
                }
                if h.kind == GraphicsKind::Baseline && a.observe_baseline(&h.object).is_ok() {
                    if found.is_some() {
                        return Err("Ambiguous duplicate graphics identity".into());
                    }
                    found = Some(Located {
                        figure: id,
                        position: Some(position),
                        series: None,
                        textbox: None,
                    });
                }
                if h.kind == GraphicsKind::Series {
                    for (index, s) in a.series.iter().enumerate() {
                        if h.object.same_object(&s.identity) {
                            if found.is_some() {
                                return Err("Ambiguous duplicate graphics identity".into());
                            }
                            found = Some(Located {
                                figure: id,
                                position: Some(position),
                                series: Some(index),
                                textbox: None,
                            });
                        }
                    }
                }
            }
        }
        found.ok_or("Graphics handle is stale or does not belong to this engine graph".into())
    }
    fn graphics_figure(&self, id: u32) -> Result<&Figure> {
        self.figures()
            .find(|(n, _)| *n == id)
            .map(|(_, f)| f)
            .ok_or("Graphics figure is no longer live".into())
    }
    fn publish_graphics_figure(&mut self, id: u32, figure: Figure) {
        if id == self.current_figure {
            self.figure = Some(figure);
        } else {
            self.other_figures.insert(id, figure);
        }
    }
    pub(super) fn shared_title_graphics(&mut self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let (id, args) = if matches!(args.first(), Some(Value::Container(Array::Graphics(_)))) {
            let target = handle(&args[0])?;
            if target.kind() != GraphicsKind::Figure {
                return Err("sgtitle target must be a live figure handle".into());
            }
            (self.locate_graphics(target)?.figure, &args[1..])
        } else {
            (self.current_figure, args)
        };
        let Some((title, properties)) = args.split_first() else {
            return Err("Use sgtitle([figure,] text, 'Interpreter', 'none')".into());
        };
        let title = text(title)?;
        if !properties.len().is_multiple_of(2) {
            return Err("sgtitle properties require name/value pairs".into());
        }
        for pair in properties.as_chunks::<2>().0 {
            if !text(&pair[0])?.eq_ignore_ascii_case("Interpreter")
                || !text(&pair[1])?.eq_ignore_ascii_case("none")
            {
                return Err("sgtitle supports literal text with Interpreter none; other text properties are unsupported".into());
            }
        }
        let mut candidate = if id == self.current_figure {
            self.figure.clone().unwrap_or_default()
        } else {
            self.graphics_figure(id)?.clone()
        };
        candidate.set_shared_title(title.to_owned())?;
        candidate.validate()?;
        #[cfg(test)]
        tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(id, candidate);
        Ok(Value::Void)
    }
    pub(super) fn graphics_series_result(&self, figure: &Figure, first: usize) -> Result<Value> {
        let selected = figure
            .axes
            .series
            .get(first..)
            .ok_or("Invalid new series range")?;
        let mut handles = Vec::new();
        handles
            .try_reserve_exact(selected.len())
            .map_err(|_| "Cannot allocate series handles")?;
        for series in selected {
            self.check_cancel()?;
            handles.push(Arc::new(GraphicsHandle::series(
                &figure.identity,
                &figure.axes.identity,
                &series.identity,
            )));
        }
        Ok(Value::Container(
            GraphicsArray::from_shared(selected.len(), 1, handles, &self.cancel)?.into(),
        ))
    }
    // Read-only target resolution: no selection, identity replacement or numeric data copy.
    pub(super) fn export_graphics_target(
        &self,
        target: &Value,
    ) -> Result<(&Figure, Option<Position>)> {
        self.check_cancel()?;
        let (number, position) = match target {
            Value::Container(Array::Graphics(handles)) => {
                let h = handles.scalar_handle()?;
                if !matches!(h.kind(), GraphicsKind::Figure | GraphicsKind::Axes) {
                    return Err(
                        "exportgraphics target must be one live Figure or Axes handle".into(),
                    );
                }
                let located = self.locate_graphics(h)?;
                (located.figure, located.position)
            }
            Value::Matrix(id) => {
                let value = id.as_scalar()?;
                if !value.is_finite() || value.fract() != 0. || !(1. ..=1_000_000.).contains(&value)
                {
                    return Err(
                        "exportgraphics requires one existing integer figure ID in 1..1000000"
                            .into(),
                    );
                }
                (value as u32, None)
            }
            _ => return Err(
                "exportgraphics target must be one live Figure/Axes handle or numeric figure ID"
                    .into(),
            ),
        };
        Ok((self.graphics_figure(number)?, position))
    }
    pub(super) fn graphics_file_arguments(&self, args: Vec<Value>) -> Result<Vec<Value>> {
        let mut args = args;
        if let Some(Value::Container(Array::Graphics(a))) = args.first() {
            let h = a.scalar_handle()?;
            if h.kind() != GraphicsKind::Figure {
                return Err("File export requires a figure handle".into());
            }
            let located = self.locate_graphics(h)?;
            args[0] = Value::Matrix(Matrix::scalar(f64::from(located.figure)));
        }
        Ok(args)
    }
    pub(super) fn graphics_handle_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let has_leading_handle = matches!(args.first(), Some(Value::Container(Array::Graphics(_))));
        if !matches!(
            name,
            "figure"
                | "gcf"
                | "gca"
                | "axes"
                | "get"
                | "set"
                | "isgraphics"
                | "patch"
                | "annotation"
                | "sgtitle"
                | "legend"
        ) && !has_leading_handle
        {
            return Ok(None);
        }
        self.check_cancel()?;
        let args = self.text_arguments(args.to_vec())?;
        if name == "sgtitle" {
            return self.shared_title_graphics(&args).map(Some);
        }
        if name == "legend" {
            return self.legend_object_call(&args).map(Some);
        }
        if name == "patch" {
            return self.create_patch_object(&args).map(Some);
        }
        if name == "annotation" {
            return self.create_textbox_object(&args).map(Some);
        }
        if name == "isgraphics" {
            if args.len() != 1 {
                return Err("isgraphics initially accepts one handle array".into());
            }
            let Value::Container(Array::Graphics(h)) = &args[0] else {
                return Err("isgraphics currently requires a typed handle array; numeric handle encodings are unsupported".into());
            };
            let mut values = Vec::new();
            values
                .try_reserve_exact(h.len())
                .map_err(|_| "Cannot allocate graphics validity result")?;
            for i in 0..h.len() {
                self.check_cancel()?;
                let live = self.locate_graphics(h.get_linear(i)?).is_ok();
                self.check_cancel()?; // Cancellation cannot become a false validity answer.
                values.push(u8::from(live));
            }
            return Ok(Some(Value::Logical(LogicalMatrix::from_column_vec(
                h.rows(),
                h.cols(),
                values,
                &self.cancel,
            )?)));
        }
        if matches!(name, "figure" | "gcf" | "gca") {
            if name != "figure" && !args.is_empty() {
                return Err(format!("{name} takes no arguments"));
            }
            let property_constructor =
                name == "figure" && matches!(args.first(), Some(Value::Text(_)));
            let properties = if property_constructor {
                args.as_slice()
            } else {
                &[]
            };
            if !property_constructor && args.len() > 1 {
                return Err("Use figure(Name,Value), figure(number) or figure(handle), with set for existing figure properties".into());
            }
            if properties.len() > 32 || !properties.len().is_multiple_of(2) {
                return Err("Figure accepts at most 16 property/value pairs".into());
            }
            let id = if property_constructor {
                (1..=17)
                    .find(|id| !self.figure_ids().contains(id))
                    .ok_or("No free figure number")?
            } else if name == "figure" {
                match args.first() {
                    None => (1..=17)
                        .find(|id| !self.figure_ids().contains(id))
                        .ok_or("No free figure number")?,
                    Some(Value::Container(Array::Graphics(h))) => {
                        let h = h.scalar_handle()?;
                        if h.kind() != GraphicsKind::Figure {
                            return Err("figure requires a figure handle".into());
                        }
                        self.locate_graphics(h)?.figure
                    }
                    Some(value) => {
                        let n = value.clone().matrix(&self.cancel)?.as_scalar()?;
                        if !n.is_finite() || n.fract() != 0. || !(1. ..=1_000_000.).contains(&n) {
                            return Err("Figure number must be an integer in 1..1000000".into());
                        }
                        n as u32
                    }
                }
            } else {
                self.current_figure
            };
            let mut figure = self
                .figures()
                .find(|(number, _)| *number == id)
                .map(|(_, f)| f.clone())
                .unwrap_or_default();
            for pair in properties.as_chunks::<2>().0 {
                figure_appearance(&mut figure, pair, self.graphics_theme_context)?;
            }
            if name == "gca" {
                figure.activate_tile();
            }
            let h = if name == "gca" {
                GraphicsHandle::axes(&figure.identity, &figure.axes.identity)
            } else {
                GraphicsHandle::figure(&figure.identity)
            };
            let result = GraphicsArray::scalar(h, &self.cancel)?;
            figure.validate()?;
            #[cfg(test)]
            tests::before_property_publish(&self.cancel);
            self.check_cancel()?;
            self.select_figure(id)?;
            self.figure = Some(figure);
            return Ok(Some(Value::Container(result.into())));
        }
        if name == "axes" {
            if args.len() == 1 && has_leading_handle {
                let h = handle(&args[0])?;
                if h.kind() != GraphicsKind::Axes {
                    return Err("axes selection requires an axes handle".into());
                }
                let located = self.locate_graphics(h)?;
                let mut figure = self.graphics_figure(located.figure)?.clone();
                select_live_axes(&mut figure, located.position.unwrap())?;
                let result = GraphicsArray::scalar(h.clone(), &self.cancel)?;
                self.check_cancel()?;
                self.select_figure(located.figure)?;
                self.figure = Some(figure);
                return Ok(Some(Value::Container(result.into())));
            }
            if args.len() > 32 || !args.len().is_multiple_of(2) {
                return Err("axes accepts at most 16 property/value pairs".into());
            }
            let mut parent = self.current_figure;
            let mut position = [0.13, 0.11, 0.775, 0.815];
            let mut hold = false;
            for pair in args.as_chunks::<2>().0 {
                match text(&pair[0])?.to_ascii_lowercase().as_str() {
                    "parent" => {
                        let h = handle(&pair[1])?;
                        if h.kind() != GraphicsKind::Figure {
                            return Err("Axes Parent must be a figure handle".into());
                        }
                        parent = self.locate_graphics(h)?.figure;
                    }
                    "units" if text(&pair[1])?.eq_ignore_ascii_case("normalized") => {}
                    "position" => position = vector(&pair[1], 4)?.try_into().unwrap(),
                    "nextplot" => {
                        hold = match text(&pair[1])? {
                            "add" => true,
                            "replace" => false,
                            _ => return Err("Axes NextPlot supports add or replace".into()),
                        }
                    }
                    _ => return Err(
                        "Axes constructor supports Parent, normalized Units, Position and NextPlot"
                            .into(),
                    ),
                }
            }
            let position = Position::from_normalized(position)?;
            let mut figure = self
                .figures()
                .find(|(id, _)| *id == parent)
                .map(|(_, f)| f.clone())
                .unwrap_or_default();
            // The plot model has one implicit empty default axes. Creating the
            // first explicit axes replaces it; subsequent overlapping axes are
            // unsupported and reject without deleting existing objects.
            let implicit = figure.layout.is_none()
                && figure.axes.series.is_empty()
                && figure.axes.title.is_empty()
                && figure.axes.xlabel.is_empty()
                && figure.axes.ylabel.is_empty();
            if !implicit && figure.all_axes().any(|(p, _)| p.overlaps(position)) {
                return Err(
                    "Overlapping axes creation is not supported by the current 2-D model".into(),
                );
            }
            figure.select_subplot(position, true)?;
            figure.axes.hold = hold;
            figure.validate()?;
            let result = GraphicsArray::scalar(
                GraphicsHandle::axes(&figure.identity, &figure.axes.identity),
                &self.cancel,
            )?;
            self.check_cancel()?;
            self.select_figure(parent)?;
            self.figure = Some(figure);
            return Ok(Some(Value::Container(result.into())));
        }
        if matches!(name, "get" | "set") {
            let target = args
                .first()
                .ok_or("Graphics property operation requires a handle")?;
            let h = handle(target)?;
            let located = self.locate_graphics(h)?;
            return if name == "get" {
                self.get_graphics_property(h, located, &args[1..]).map(Some)
            } else {
                self.set_graphics_properties(h, located, &args[1..])?;
                Ok(Some(Value::Void))
            };
        }
        if name == "close" && has_leading_handle {
            if args.len() != 1 || handle(&args[0])?.kind() != GraphicsKind::Figure {
                return Err("close requires one figure handle".into());
            }
            let id = self.locate_graphics(handle(&args[0])?)?.figure;
            return self
                .graphics("close", vec![Value::Matrix(Matrix::scalar(f64::from(id)))])
                .map(Some);
        }
        if has_leading_handle
            && matches!(
                name,
                "plot"
                    | "semilogx"
                    | "semilogy"
                    | "loglog"
                    | "scatter"
                    | "bar"
                    | "stairs"
                    | "stem"
                    | "title"
                    | "xlabel"
                    | "ylabel"
                    | "hold"
                    | "grid"
                    | "xlim"
                    | "ylim"
            )
        {
            let h = handle(&args[0])?;
            if h.kind() != GraphicsKind::Axes {
                return Err("Leading graphics target must be an axes handle".into());
            }
            let located = self.locate_graphics(h)?;
            let mut selected = self.graphics_figure(located.figure)?.clone();
            select_live_axes(&mut selected, located.position.unwrap())?;
            // Roll back metadata/current selection on any downstream validation,
            // allocation or Stop error. Sample buffers remain immutable/shared.
            let old = (
                self.current_figure,
                self.figure.clone(),
                self.other_figures.clone(),
            );
            let result = (|| {
                self.select_figure(located.figure)?;
                self.figure = Some(selected);
                let tail = args[1..].to_vec();
                let value = self.call(name, tail, out)?;
                self.check_cancel()?;
                Ok(value)
            })();
            if result.is_err() {
                (self.current_figure, self.figure, self.other_figures) = old;
            }
            return result.map(Some);
        }
        Ok(None)
    }

    fn get_graphics_property(
        &self,
        h: &GraphicsHandle,
        located: Located,
        args: &[Value],
    ) -> Result<Value> {
        if args.len() != 1 {
            return Err("get initially requires one explicit property name".into());
        }
        if h.kind() == GraphicsKind::Baseline {
            return self.get_baseline_property(h, located, args);
        }
        if h.kind() == GraphicsKind::Legend {
            return self.get_legend_property(located, args);
        }
        if let Some(index) = located.textbox {
            return self.get_textbox_property(located.figure, index, args);
        }
        let name = text(&args[0])?.to_ascii_lowercase();
        let figure = self.graphics_figure(located.figure)?;
        let string = |s: &str| Ok(Value::Text(s.into()));
        if name == "type" && h.kind() != GraphicsKind::Series {
            return string(if h.kind() == GraphicsKind::Figure {
                "figure"
            } else {
                "axes"
            });
        }
        if h.kind() == GraphicsKind::Figure {
            return match name.as_str() {
                "name" => string(&figure.name),
                // Predeclared legacy compatibility default; this does not invert exports.
                "inverthardcopy" => string(if figure.invert_hardcopy.unwrap_or(true) { "on" } else { "off" }),
                "theme" => string(figure.resolve_appearance(self.graphics_theme_context).theme.as_str()),
                "thememode" => string(if figure.appearance_policy().theme == crate::plot::FigureTheme::Auto { "auto" } else { "manual" }),
                "colormode" => string(figure.appearance_policy().canvas_mode.as_str()),
                "color" => Ok(Value::Matrix(Matrix::from_column_slice(
                    1,
                    3,
                    &figure.resolve_appearance(self.graphics_theme_context).canvas.map(|c| f64::from(c) / 255.),
                )?)),
                "number" => Ok(Value::Matrix(Matrix::scalar(f64::from(located.figure)))),
                "currentaxes" => Ok(Value::Container(
                    GraphicsArray::scalar(
                        GraphicsHandle::axes(&figure.identity, &figure.axes.identity),
                        &self.cancel,
                    )?
                    .into(),
                )),
                _ => Err(
                    "Supported figure queries: Type, Number, CurrentAxes, Name, Color, Theme, ThemeMode, ColorMode and InvertHardcopy".into(),
                ),
            };
        }
        let (_, axes) = figure
            .all_axes()
            .find(|(p, _)| Some(*p) == located.position)
            .ok_or("Axes is no longer live")?;
        if name == "parent" {
            let parent = if h.kind() == GraphicsKind::Axes {
                GraphicsHandle::figure(&figure.identity)
            } else {
                GraphicsHandle::axes(&figure.identity, &axes.identity)
            };
            return Ok(Value::Container(
                GraphicsArray::scalar(parent, &self.cancel)?.into(),
            ));
        }
        if let Some(index) = located.series {
            let s = &axes.series[index];
            if matches!(
                s.kind,
                crate::plot::PlotKind::Bar | crate::plot::PlotKind::Stem
            ) {
                let baseline = axes.observe_series_baseline(index)?;
                match name.as_str() {
                    "baseline" => {
                        return Ok(Value::Container(
                            GraphicsArray::scalar(
                                GraphicsHandle::baseline(
                                    &figure.identity,
                                    &axes.identity,
                                    baseline.identity,
                                ),
                                &self.cancel,
                            )?
                            .into(),
                        ));
                    }
                    "showbaseline" => {
                        return string(if baseline.style.visible { "on" } else { "off" });
                    }
                    "basevalue" => {
                        return Ok(Value::Matrix(Matrix::scalar(
                            axes.baseline_value_cancellable(baseline.identity, &self.cancel)?,
                        )));
                    }
                    _ => {}
                }
            }
            return match name.as_str() {
                "type" if s.kind==crate::plot::PlotKind::Line=>string("line"),
                "type" if s.kind==crate::plot::PlotKind::Patch=>string("patch"),
                "type" if s.kind==crate::plot::PlotKind::Bar=>string("bar"),
                "type" if s.kind==crate::plot::PlotKind::Scatter=>string("scatter"),
                "type" if s.kind==crate::plot::PlotKind::Stairs=>string("stair"),
                "type" if s.kind==crate::plot::PlotKind::Stem=>string("stem"),
                "facecolor" if matches!(s.kind,crate::plot::PlotKind::Patch | crate::plot::PlotKind::Bar)=>Ok(Value::Matrix(Matrix::from_column_slice(1,3,&figure.resolve_appearance(self.graphics_theme_context).series_color(s).map(|c|f64::from(c)/255.))?)),
                "edgecolor" if matches!(s.kind,crate::plot::PlotKind::Patch | crate::plot::PlotKind::Bar)=>match s.face_edge_color {None=>string("none"),Some(rgb)=>Ok(Value::Matrix(Matrix::from_column_slice(1,3,&rgb.map(|c|f64::from(c)/255.))?))},
                "linestyle"=>string(match s.pattern {crate::plot::LinePattern::Solid=>"-",crate::plot::LinePattern::Dashed=>"--",crate::plot::LinePattern::Dotted=>":",crate::plot::LinePattern::DashDot=>"-.",crate::plot::LinePattern::None=>"none"}),
                "displayname"=>string(&s.label),"visible"=>string(if s.visible {"on"} else {"off"}),
                "linewidth" if matches!(s.kind,crate::plot::PlotKind::Bar|crate::plot::PlotKind::Patch)=>Ok(Value::Matrix(Matrix::scalar(s.line_width.unwrap_or(0.5)))),
                "linewidth"=>s.line_width.map(|width|Value::Matrix(Matrix::scalar(width))).ok_or("Implicit legacy line width has no explicit stored property; set LineWidth before querying it".into()),
                _=>Err("Supported series queries: Type, Parent, DisplayName, Visible and LineWidth".into()),
            };
        }
        match name.as_str() {
            "color" | "xcolor" | "ycolor" => {
                let stored = axes.colors.unwrap_or_default();
                let resolved =
                    axes.resolve_colors(figure.resolve_appearance(self.graphics_theme_context));
                let (rgb, fallback) = match name.as_str() {
                    "color" => (stored.background, resolved.background),
                    "xcolor" => (stored.x, resolved.x),
                    _ => (stored.y, resolved.y),
                };
                Ok(Value::Matrix(Matrix::from_column_slice(
                    1,
                    3,
                    &rgb.unwrap_or_else(|| fallback.map(|v| f64::from(v) / 255.)),
                )?))
            }
            "gridcolor" | "gridalpha" | "xcolormode" | "ycolormode" | "gridcolormode"
            | "gridalphamode" => {
                let grid = axes.colors.and_then(|c| c.grid).unwrap_or_else(|| {
                    crate::plot::AxesGridStyle::from_appearance(
                        figure.resolve_appearance(self.graphics_theme_context),
                        !figure
                            .resolve_appearance(self.graphics_theme_context)
                            .legacy,
                    )
                });
                match name.as_str() {
                    "gridcolor" => Ok(Value::Matrix(Matrix::from_column_slice(1, 3, &grid.color)?)),
                    "gridalpha" => Ok(Value::Matrix(Matrix::scalar(grid.alpha))),
                    "xcolormode" => string(grid.x_mode.as_str()),
                    "ycolormode" => string(grid.y_mode.as_str()),
                    "gridcolormode" => string(grid.color_mode.as_str()),
                    _ => string(grid.alpha_mode.as_str()),
                }
            }
            "box" => string("on"),
            "nextplot" => string(if axes.hold { "add" } else { "replace" }),
            "xgrid" => string(if axes.xgrid { "on" } else { "off" }),
            "ygrid" => string(if axes.ygrid { "on" } else { "off" }),
            "xscale" => string(if axes.xscale == crate::plot::AxisScale::Log {
                "log"
            } else {
                "linear"
            }),
            "yscale" => string(if axes.yscale == crate::plot::AxisScale::Log {
                "log"
            } else {
                "linear"
            }),
            "position" => {
                let rect = figure
                    .normalized_axes_positions()
                    .into_iter()
                    .find(|(p, _)| Some(*p) == located.position)
                    .unwrap()
                    .1;
                Ok(Value::Matrix(Matrix::from_column_slice(1, 4, &rect)?))
            }
            "xlim" | "ylim" => {
                let (x, y) = axes.bounds();
                Ok(Value::Matrix(Matrix::from_column_slice(
                    1,
                    2,
                    if name == "xlim" { &x } else { &y },
                )?))
            }
            _ => Err("Unsupported axes property query".into()),
        }
    }
    fn set_graphics_properties(
        &mut self,
        h: &GraphicsHandle,
        located: Located,
        args: &[Value],
    ) -> Result<()> {
        if args.is_empty() || args.len() > 32 || !args.len().is_multiple_of(2) {
            return Err("set requires 1..16 property/value pairs".into());
        }
        if h.kind() == GraphicsKind::Baseline {
            return self.set_baseline_properties(h, located, args);
        }
        if h.kind() == GraphicsKind::Legend {
            return self.set_legend_properties(located, args);
        }
        if let Some(index) = located.textbox {
            return self.set_textbox_properties(located.figure, index, args);
        }
        let mut figure = self.graphics_figure(located.figure)?.clone();
        if h.kind() == GraphicsKind::Figure {
            for pair in args.as_chunks::<2>().0 {
                if !text(&pair[0])?.eq_ignore_ascii_case("CurrentAxes") {
                    figure_appearance(&mut figure, pair, self.graphics_theme_context)?;
                    continue;
                }
                let axes = handle(&pair[1])?;
                let target = self.locate_graphics(axes)?;
                if axes.kind() != GraphicsKind::Axes || target.figure != located.figure {
                    return Err(
                        "CurrentAxes must be a live axes belonging to the same figure".into(),
                    );
                }
                select_live_axes(&mut figure, target.position.unwrap())?;
            }
        } else {
            let previous = figure.axes.identity.clone();
            select_live_axes(&mut figure, located.position.unwrap())?;
            if let Some(index) = located.series {
                let kind = figure.axes.series[index].kind;
                if matches!(
                    kind,
                    crate::plot::PlotKind::Bar | crate::plot::PlotKind::Stem
                ) {
                    self.set_chart_properties(&mut figure.axes, index, args)?;
                } else if kind == crate::plot::PlotKind::Patch {
                    object_commands::patch_properties(&mut figure.axes.series[index], args)?;
                } else {
                    let (start, properties) =
                        super::plot_properties::Properties::parse(args, kind)?;
                    if start != 0 {
                        return Err("set requires property/value pairs only".into());
                    }
                    properties.apply(&mut figure.axes.series[index..index + 1], kind)?;
                }
            } else {
                let mut placement = figure.initial_axes_placement()?;
                let mut change_placement = false;
                let mut modern = None;
                let mut legacy = None;
                for pair in args.as_chunks::<2>().0 {
                    match text(&pair[0])?.to_ascii_lowercase().as_str() {
                        "color" | "xcolor" | "ycolor" | "gridcolor" | "gridalpha"
                        | "xcolormode" | "ycolormode" | "gridcolormode" | "gridalphamode" => {
                            set_axes_color(
                                &mut figure,
                                pair,
                                self.graphics_theme_context,
                                &self.cancel,
                            )?;
                        }
                        "xgrid" | "ygrid" => {
                            let enabled = match &pair[1] {
                                Value::Text(value) if value.eq_ignore_ascii_case("on") => true,
                                Value::Text(value) if value.eq_ignore_ascii_case("off") => false,
                                Value::Logical(value) => value.as_scalar()?,
                                Value::Matrix(value) if value.as_scalar()? == 1. => true,
                                Value::Matrix(value) if value.as_scalar()? == 0. => false,
                                _ => {
                                    return Err(
                                        "Grid visibility requires on, off, logical or numeric 0/1"
                                            .into(),
                                    );
                                }
                            };
                            if text(&pair[0])?.eq_ignore_ascii_case("XGrid") {
                                figure.axes.xgrid = enabled;
                            } else {
                                figure.axes.ygrid = enabled;
                            }
                        }
                        "box" if text(&pair[1])?.eq_ignore_ascii_case("on") => {}
                        "nextplot" => {
                            figure.axes.hold = match text(&pair[1])? {
                                "add" => true,
                                "replace" => false,
                                _ => return Err("NextPlot supports add or replace".into()),
                            }
                        }
                        "xscale" | "yscale" => {
                            let scale = match text(&pair[1])? {
                                "linear" => crate::plot::AxisScale::Linear,
                                "log" => crate::plot::AxisScale::Log,
                                _ => return Err("Scale must be linear or log".into()),
                            };
                            if text(&pair[0])?.eq_ignore_ascii_case("XScale") {
                                figure.axes.xscale = scale;
                            } else {
                                figure.axes.yscale = scale;
                            }
                        }
                        "xlim" | "ylim" => {
                            let values: [f64; 2] = vector(&pair[1], 2)?.try_into().unwrap();
                            crate::plot::validate_limits(values)?;
                            if text(&pair[0])?.eq_ignore_ascii_case("XLim") {
                                figure.axes.xlim = Some(values);
                            } else {
                                figure.axes.ylim = Some(values);
                            }
                        }
                        "units" if text(&pair[1])?.eq_ignore_ascii_case("normalized") => {}
                        "position" | "innerposition" => {
                            placement.requested_inner = vector(&pair[1], 4)?.try_into().unwrap();
                            change_placement = true;
                        }
                        "outerposition" => {
                            placement.requested_outer = vector(&pair[1], 4)?.try_into().unwrap();
                            change_placement = true;
                        }
                        "positionconstraint" => {
                            modern = Some(PositionConstraint::parse(text(&pair[1])?)?);
                            change_placement = true;
                        }
                        "activepositionproperty" => {
                            legacy =
                                Some(match text(&pair[1])? {
                                    "position" => PositionConstraint::InnerPosition,
                                    "outerposition" => PositionConstraint::OuterPosition,
                                    _ => return Err(
                                        "ActivePositionProperty must be position or outerposition"
                                            .into(),
                                    ),
                                });
                            change_placement = true;
                        }
                        _ => {
                            return Err(
                                "Unsupported axes property; no property transaction was published"
                                    .into(),
                            );
                        }
                    }
                }
                if modern.is_some() && legacy.is_some() && modern != legacy {
                    return Err("Conflicting axes position constraints".into());
                }
                if let Some(value) = modern.or(legacy) {
                    placement.constraint = value;
                }
                if change_placement {
                    figure.set_axes_placement(placement)?;
                }
            }
            // set does not select another axes. Locate the prior identity after
            // a possible geometry change rather than reusing its old rectangle.
            let previous_position = figure
                .all_axes()
                .find(|(_, a)| a.identity.same_object(&previous))
                .map(|(p, _)| p)
                .ok_or("Property update removed the previous axes")?;
            select_live_axes(&mut figure, previous_position)?;
        }
        figure.prepare_legend_layout()?;
        figure.validate()?;
        #[cfg(test)]
        tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(located.figure, figure);
        Ok(())
    }
}

/// Preserve normalized RGB instead of quantizing document properties to bytes.
pub(super) fn axes_rgb(value: &Value, cancel: &std::sync::atomic::AtomicBool) -> Result<[f64; 3]> {
    let shape = match value {
        Value::Matrix(m) => (m.rows(), m.cols()),
        Value::Logical(m) => (m.rows(), m.cols()),
        _ => return Err("Axes/text Color requires a real numeric or logical RGB vector".into()),
    };
    if shape != (1, 3) && shape != (3, 1) {
        return Err("Axes/text Color requires a three-element real RGB vector".into());
    }
    let matrix = value.clone().matrix(cancel)?;
    let rgb: [f64; 3] = matrix.data().try_into().unwrap();
    crate::plot::AxesColors::validate_rgb(rgb)?;
    Ok(rgb)
}

fn set_axes_color(
    figure: &mut Figure,
    pair: &[Value],
    host: crate::plot::BaseTheme,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    use crate::plot::{AxesColors, AxesGridStyle, ColorMode};
    let name = text(&pair[0])?.to_ascii_lowercase();
    let appearance = figure.resolve_appearance(host);
    let modern = !appearance.legacy;
    let colors = figure.axes.colors.get_or_insert_with(Default::default);
    if name == "color" {
        colors.background = Some(axes_rgb(&pair[1], cancel)?);
        return Ok(());
    }
    let grid = colors
        .grid
        .get_or_insert_with(|| AxesGridStyle::from_appearance(appearance, modern));
    match name.as_str() {
        "xcolor" | "ycolor" => {
            let rgb = axes_rgb(&pair[1], cancel)?;
            if name == "xcolor" {
                colors.x = Some(rgb);
                grid.x_mode = ColorMode::Manual;
            } else {
                colors.y = Some(rgb);
                grid.y_mode = ColorMode::Manual;
            }
        }
        "gridcolor" => {
            grid.color = axes_rgb(&pair[1], cancel)?;
            grid.color_mode = ColorMode::Manual;
        }
        "gridalpha" => {
            let shape = match &pair[1] {
                Value::Matrix(m) => (m.rows(), m.cols()),
                Value::Logical(m) => (m.rows(), m.cols()),
                _ => return Err("GridAlpha requires a real numeric or logical scalar".into()),
            };
            if shape != (1, 1) {
                return Err("GridAlpha requires one scalar".into());
            }
            let alpha = pair[1].clone().matrix(cancel)?.as_scalar()?;
            if !alpha.is_finite() || !(0. ..=1.).contains(&alpha) {
                return Err("GridAlpha requires a finite value in [0,1]".into());
            }
            grid.alpha = alpha;
            grid.alpha_mode = ColorMode::Manual;
        }
        _ => {
            let mode = match text(&pair[1])?.to_ascii_lowercase().as_str() {
                "auto" => ColorMode::Auto,
                "manual" => ColorMode::Manual,
                _ => return Err("Axes color modes require auto or manual".into()),
            };
            match name.as_str() {
                "xcolormode" => {
                    grid.x_mode = mode;
                    if mode == ColorMode::Manual {
                        colors
                            .x
                            .get_or_insert(appearance.axis.map(|v| f64::from(v) / 255.));
                    }
                }
                "ycolormode" => {
                    grid.y_mode = mode;
                    if mode == ColorMode::Manual {
                        colors
                            .y
                            .get_or_insert(appearance.axis.map(|v| f64::from(v) / 255.));
                    }
                }
                "gridcolormode" => grid.color_mode = mode,
                "gridalphamode" => {
                    grid.alpha_mode = mode;
                    if mode == ColorMode::Auto {
                        grid.alpha = 0.15;
                    }
                }
                _ => return Err("Unsupported axes color property".into()),
            }
        }
    }
    // Validate the complete candidate before the outer graphics transaction commits.
    AxesColors::validate(*colors)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::{
        cell::Cell,
        sync::atomic::{AtomicBool, Ordering},
    };
    thread_local! {static STOP_BEFORE_PUBLISH: Cell<bool> = const {Cell::new(false)};}
    pub(in crate::engine) fn before_property_publish(cancel: &AtomicBool) {
        if STOP_BEFORE_PUBLISH.with(|armed| armed.replace(false)) {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            STOP_BEFORE_PUBLISH.with(|armed| armed.set(false));
        }
    }
    #[test]
    fn baseline_handle_set_cancels_at_final_publication_without_materialization() {
        let mut e = Engine::default();
        e.execute("f=figure(3);s=stem([1 2],[3 4]);b=get(s,'BaseLine');figure(9);")
            .unwrap();
        let before = e.figures().find(|(id, _)| *id == 3).unwrap().1.clone();
        let workspace = e.workspace();
        let _reset = Reset;
        STOP_BEFORE_PUBLISH.with(|armed| armed.set(true));
        assert!(
            e.execute("set(b,'Color',[1 0 0],'Visible','off','BaseValue',-2);")
                .is_err()
        );
        assert!(before.same_snapshot(e.figures().find(|(id, _)| *id == 3).unwrap().1));
        assert_eq!(e.current_figure, 9);
        for (name, value) in workspace {
            assert!(value.same_snapshot(&e.array(&name).unwrap()));
        }
        e.cancel.store(false, Ordering::Relaxed);
        e.execute("set(b,'Color',[1 0 0],'Visible','off','BaseValue',-2);")
            .unwrap();
        let changed = e.figures().find(|(id, _)| *id == 3).unwrap().1;
        assert_eq!(changed.baselines.len(), 1);
        assert_eq!(changed.series[0].base_value, Some(-2.));
        assert_eq!(e.current_figure, 9);
    }
    #[test]
    fn appearance_and_literal_label_updates_cancel_before_publication() {
        for script in [
            "fresh=figure('Name','private','Color',[1 0 0]);",
            "set(f,'Name','private','Color',[1 0 0]);",
            "set(h,'BaseValue',-3,'MarkerFaceColor','none');",
            "title(a,'private','Interpreter','none');",
            "set(a,'Color',[1 0 0],'XColor',[0 1 0]);",
            "title(a,'private','Color',[1 0 0]);",
            "xlabel(a,'private','Interpreter','none','Color',[0 1 0]);",
            "sgtitle(f,'private','Interpreter','none');",
        ] {
            let mut e = Engine::default();
            e.execute("f=figure('Name','original');a=gca;h=stem(a,[0 1],[2 3],'Color',[0 0 1],'Marker','o','MarkerFaceColor',[0 0 1]);figure(9);").unwrap();
            let original: Vec<_> = e.figures().map(|(n, f)| (n, f.clone())).collect();
            let variables = e.workspace();
            let _reset = Reset;
            STOP_BEFORE_PUBLISH.with(|armed| armed.set(true));
            assert!(
                e.execute(script)
                    .unwrap_err()
                    .to_ascii_lowercase()
                    .contains("cancel")
            );
            assert!(!STOP_BEFORE_PUBLISH.with(Cell::get));
            assert_eq!(e.current_figure, 9);
            assert_eq!(e.figures().count(), original.len());
            for (n, f) in &original {
                assert!(f.same_snapshot(e.figures().find(|(id, _)| id == n).unwrap().1));
            }
            for (name, a) in variables {
                assert!(a.same_snapshot(&e.array(&name).unwrap()));
            }
            e.cancel.store(false, Ordering::Relaxed);
            e.execute(script).unwrap();
            e.figures().for_each(|(_, f)| f.validate().unwrap());
        }
    }
    #[test]
    fn legend_stop_after_candidate_keeps_graph_handle_aliases_and_retry_valid() {
        for script in [
            "fresh=legend(a,[h(2) h(1)],{'second','first'},'Interpreter','none');",
            "set(lg,'String',{'renamed'},'AutoUpdate','off');",
            "legend(a,'off');",
        ] {
            let mut e = Engine::default();
            e.execute("f=figure(1);a=gca;h=plot(a,[0 1],[1 2],[0 1],[2 3]);lg=legend(a,h(1),{'old'},'Interpreter','none');alias=lg;figure(2);").unwrap();
            let before: Vec<_> = e.figures().map(|(n, f)| (n, f.clone())).collect();
            let variables = e.workspace();
            let _reset = Reset;
            STOP_BEFORE_PUBLISH.with(|armed| armed.set(true));
            assert!(
                e.execute(script)
                    .unwrap_err()
                    .to_ascii_lowercase()
                    .contains("cancel")
            );
            assert!(!STOP_BEFORE_PUBLISH.with(Cell::get));
            assert_eq!(e.current_figure, 2);
            for (n, f) in before {
                assert!(f.same_snapshot(e.figures().find(|(id, _)| *id == n).unwrap().1));
            }
            for (name, value) in variables {
                assert!(value.same_snapshot(&e.array(&name).unwrap()));
            }
            e.cancel.store(false, Ordering::Relaxed);
            e.execute(script).unwrap();
            e.figures().for_each(|(_, f)| f.validate().unwrap());
        }
    }
    #[test]
    fn patch_and_textbox_completed_candidates_cancel_without_graph_publication() {
        for script in [
            "p=patch('Parent',a,'XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor','none');",
            "b=annotation(f,'textbox',[0 .9 1 .1],'String','new','Interpreter','none','EdgeColor','none');",
            "set(t,'String','changed','FontSize',20);",
            "set(p0,'FaceColor',[0 1 0],'Visible','off');",
        ] {
            let mut engine = Engine::default();
            engine.execute("f=figure(1);a=gca;p0=patch(a,'XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor','none');t=annotation(f,'textbox',[0 .9 1 .1],'String','old','Interpreter','none','EdgeColor','none');figure(2);").unwrap();
            let before: Vec<_> = engine.figures().map(|(n, f)| (n, f.clone())).collect();
            let variables = engine.workspace();
            let _reset = Reset;
            STOP_BEFORE_PUBLISH.with(|armed| armed.set(true));
            assert!(
                engine
                    .execute(script)
                    .unwrap_err()
                    .to_ascii_lowercase()
                    .contains("cancel")
            );
            assert!(
                !STOP_BEFORE_PUBLISH.with(Cell::get),
                "Reached completed private candidate"
            );
            assert_eq!(engine.current_figure, 2);
            for (id, figure) in before {
                assert!(
                    engine
                        .figures()
                        .find(|(n, _)| *n == id)
                        .unwrap()
                        .1
                        .same_snapshot(&figure)
                );
            }
            for (name, value) in variables {
                assert!(value.same_snapshot(&engine.array(&name).unwrap()));
            }
            engine.cancel.store(false, Ordering::Relaxed);
            engine.execute(script).unwrap();
            assert_eq!(engine.current_figure, 2);
            engine.figures().for_each(|(_, f)| f.validate().unwrap());
        }
    }
    #[test]
    fn completed_property_candidate_observes_stop_before_publication_and_retries() {
        let mut engine = Engine::default();
        engine
            .execute("f=figure(1);a=gca;h=plot(a,[1 2],[3 4],'LineWidth',2);alias=h;figure(2);")
            .unwrap();
        let before: Vec<_> = engine.figures().map(|(n, f)| (n, f.clone())).collect();
        let alias = engine.array("alias").unwrap();
        let _reset = Reset;
        STOP_BEFORE_PUBLISH.with(|armed| armed.set(true));
        assert!(
            engine
                .execute("set(h,'LineWidth',4,'DisplayName','new');")
                .unwrap_err()
                .contains("cancel")
        );
        assert!(
            !STOP_BEFORE_PUBLISH.with(Cell::get),
            "Reached the completed candidate checkpoint"
        );
        assert_eq!(engine.current_figure, 2);
        for (n, figure) in &before {
            let actual = engine.figures().find(|(id, _)| id == n).unwrap().1;
            assert!(actual.same_snapshot(figure));
            assert!(actual.identity.same_object(&figure.identity));
        }
        assert!(alias.same_snapshot(&engine.array("alias").unwrap()));
        engine.cancel.store(false, Ordering::Relaxed);
        engine
            .execute("set(h,'LineWidth',4,'DisplayName','new');")
            .unwrap();
        let figure = engine.figures().find(|(id, _)| *id == 1).unwrap().1;
        assert_eq!(figure.series[0].line_width, Some(4.));
        assert_eq!(figure.series[0].label, "new");
        assert!(
            (before.iter().find(|(id, _)| *id == 1).unwrap().1.series[0].points)
                .same_storage(&figure.series[0].points)
        );
    }
}
