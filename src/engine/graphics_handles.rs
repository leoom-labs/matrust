//! Genuine typed graphics references. This initial layer does not implement
//! arbitrary figure colors/names or explicit legend objects.
mod baseline_commands;
mod delete_lines;
pub(super) use delete_lines::{graphics_delete_lines, remove_axes};
pub(super) mod dot_syntax;
mod find_objects;
mod image_commands;
pub(super) use dot_syntax::{gobj_bare, gobj_statement, plot_apply_color_order, plot_carry_over};
mod indexed_mesh;
mod legend_commands;
mod object_commands;
mod plot3d_commands;
mod shapes_commands;
mod surface_commands;
use super::{Engine, Execution, Value};
use crate::plot::{Figure, Position, PositionConstraint};
use crate::{Array, GraphicsArray, GraphicsHandle, GraphicsKind, LogicalMatrix, Matrix, Result};
pub(super) use plot3d_commands::{parse_view, series_from_triplet, view_outputs, view_value};
use std::sync::Arc;
pub(crate) use surface_commands::SurfaceState;
pub(super) use surface_commands::{call as surface_call, sphere_coordinates, surface_names};

#[derive(Clone, Copy)]
pub(in crate::engine) struct Located {
    pub(in crate::engine) figure: u32,
    pub(in crate::engine) position: Option<Position>,
    series: Option<usize>,
    textbox: Option<usize>,
    annotation: Option<usize>,
}
fn graphics_figures(engine: &Engine) -> impl Iterator<Item = (u32, &Figure)> {
    engine
        .figures()
        .chain(engine.ui.uiaxes_figures.iter().map(|(&id, f)| (id, f)))
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

pub(super) fn handle(value: &Value) -> Result<&GraphicsHandle> {
    match value {
        Value::Container(Array::Graphics(a)) => a.scalar_handle(),
        _ => Err("Expected one typed graphics handle".into()),
    }
}
fn handle_or_root(value: &Value) -> Result<GraphicsHandle> {
    match value {
        Value::Container(Array::Graphics(a)) => Ok(a.scalar_handle()?.clone()),
        Value::Matrix(m) if m.len() == 1 && m.as_scalar().ok() == Some(0.0) => {
            Ok(GraphicsHandle::root())
        }
        _ => Err("Expected one typed graphics handle".into()),
    }
}
pub(super) fn text(value: &Value) -> Result<&str> {
    match value {
        Value::Text(s) => Ok(s),
        _ => Err("Graphics property requires character text".into()),
    }
}

pub(in crate::engine) fn preserve_string_property_values(raw: &[Value], normalized: &mut [Value]) {
    for ([_, original], [key, value]) in raw
        .as_chunks::<2>()
        .0
        .iter()
        .zip(normalized.as_chunks_mut::<2>().0)
    {
        if text(key).is_ok_and(|key| key.eq_ignore_ascii_case("string"))
            && matches!(original, Value::QuotedString(_))
        {
            *value = original.clone();
        }
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
        "theme" => {
            figure.set_figure_theme(match text(&pair[1])?.to_ascii_lowercase().as_str() {
                "light" => crate::plot::FigureTheme::Light,
                "dark" => crate::plot::FigureTheme::Dark,
                _ => return Err("Theme supports light or dark; use ThemeMode auto for host appearance".into()),
            });
            super::graphics_root::retheme_yyaxis(figure, host);
        }
        "thememode" | "colormode" => {
            let mode = match text(&pair[1])?.to_ascii_lowercase().as_str() {
                "auto" => crate::plot::ColorMode::Auto,
                "manual" => crate::plot::ColorMode::Manual,
                _ => return Err("Appearance mode supports auto or manual".into()),
            };
            if text(&pair[0])?.eq_ignore_ascii_case("ThemeMode") {
                figure.set_theme_mode(mode, host);
                super::graphics_root::retheme_yyaxis(figure, host);
            } else {
                figure.set_canvas_mode(mode, host);
            }
        }
        "position" | "innerposition" | "outerposition" => {
            let v = vector(&pair[1], 4)?;
            figure.set_position_in_units([v[0], v[1], v[2], v[3]])?;
        }
        "units" => {
            figure.units = crate::plot::FigureUnits::parse(text(&pair[1])?)?;
        }
        "numbertitle" => {
            figure.number_title = match text(&pair[1])?.to_ascii_lowercase().as_str() {
                "on" => true,
                "off" => false,
                _ => return Err("NumberTitle supports on or off".into()),
            };
        }
        "resize" => {
            figure.resize = match text(&pair[1])?.to_ascii_lowercase().as_str() {
                "on" => true,
                "off" => false,
                _ => return Err("Resize supports on or off".into()),
            };
        }
        "windowstate" => {
            figure.window_state = crate::plot::WindowState::parse(text(&pair[1])?)?;
        }
        _ => return Err(
            "Figure supports Name, NumberTitle, Visible, Position, InnerPosition, OuterPosition, Units, Resize, WindowState, Color, Theme, ThemeMode, ColorMode and InvertHardcopy; unsupported properties are not discarded"
                .into(),
        ),
    }
    Ok(())
}
/// Set headings on the current axes, explicit axes or an explicit tile layout.
pub(super) fn heading_call(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    outputs: usize,
) -> Result<Vec<Value>> {
    use crate::plot::text_markup::Interpreter;

    let is_title = name == "title";
    if outputs > if is_title { 2 } else { 1 } {
        return Err(format!("{name}: too many outputs"));
    }
    engine.check_cancel()?;
    let (target, args) = if matches!(args.first(), Some(Value::Container(Array::Graphics(_)))) {
        (Some(handle(&args[0])?), &args[1..])
    } else {
        (None, args)
    };
    let is_layout = target.is_some_and(|h| h.kind() == GraphicsKind::TiledLayout);
    if target.is_some_and(|h| !matches!(h.kind(), GraphicsKind::Axes | GraphicsKind::TiledLayout)) {
        return Err(format!(
            "{name} target must be an axes or tiled layout handle"
        ));
    }
    let Some((label, tail)) = args.split_first() else {
        return Err(format!("{name} requires text"));
    };
    let (subtext, properties) = if is_title && !tail.len().is_multiple_of(2) {
        (tail.first(), &tail[1..])
    } else if tail.len().is_multiple_of(2) {
        (None, tail)
    } else {
        return Err("Subtitle properties require name/value pairs".into());
    };

    let located = target.map(|h| engine.locate_graphics(h)).transpose()?;
    let id = located.map_or(engine.current_figure, |l| l.figure);
    let mut candidate = if located.is_some() {
        engine.graphics_figure(id)?.clone()
    } else {
        engine.figure.clone().unwrap_or_default()
    };
    if let Some(position) = located.and_then(|l| l.position) {
        select_live_axes(&mut candidate, position)?;
    }
    if !is_layout {
        candidate.activate_tile();
    }
    let subtitle = if is_layout {
        &mut candidate.layout.as_mut().unwrap().subtitle
    } else {
        &mut candidate.axes.extras.subtitle
    };

    let cancel = &engine.cancel;
    let mut interpreter = Interpreter::Tex;
    let mut color = None;
    let mut title = None;
    let mut font_size = None;
    let mut bold = None;
    let mut italic = None;
    let mut normalized = engine.text_arguments(properties.to_vec())?;
    preserve_string_property_values(properties, &mut normalized);
    let properties = normalized.as_chunks::<2>().0;
    if is_title {
        for [key, value] in properties {
            match text(key)?.to_ascii_lowercase().as_str() {
                "interpreter" => interpreter = Interpreter::parse(text(value)?)?,
                "color" if !is_layout => color = Some(axes_rgb(value, cancel)?),
                "fontsize" => font_size = Some(dot_syntax::real_scalar(value, "FontSize")?),
                "fontweight" => bold = Some(dot_syntax::text_font_weight(value)?),
                "fontangle" => italic = Some(dot_syntax::text_font_angle(value)?),
                _ => {
                    return Err(
                        "Title supports Interpreter, axes RGB Color, FontSize, FontWeight and FontAngle"
                            .into(),
                    );
                }
            }
        }
        // Preserve the scalar character-row representation of targeted titles.
        title = Some(
            if target.is_some()
                && let Value::Container(Array::Character(chars)) = label
                && chars.is_text_row()
            {
                (chars.to_utf8(cancel)?, None)
            } else {
                super::graphics_text_lines(label, cancel)?
            },
        );
    }
    if let Some(subtext) = if is_title { subtext } else { Some(label) } {
        (subtitle.string, subtitle.string_lines) = super::graphics_text_lines(subtext, cancel)?;
        subtitle.interpreter = interpreter;
        if is_title {
            subtitle.color = color.or(subtitle.color);
            subtitle.font_size = font_size.or(subtitle.font_size);
            subtitle.bold = bold.unwrap_or(subtitle.bold);
            subtitle.italic = italic.unwrap_or(subtitle.italic);
        } else {
            for pair in properties {
                dot_syntax::apply_subtitle_property(subtitle, pair, cancel)?;
            }
        }
    }

    let (title_handle, subtitle_handle) = if is_layout {
        let layout = candidate.layout.as_mut().unwrap();
        if let Some((string, lines)) = title {
            layout.title = string;
            layout.text_lines[0] = lines;
            layout.text_interpreters[0] = interpreter;
            layout.title_style.font_size = font_size.or(layout.title_style.font_size);
            layout.title_style.bold = bold.or(layout.title_style.bold);
            layout.title_style.italic = italic.or(layout.title_style.italic);
        }
        (
            GraphicsHandle::layout_text(&candidate.identity, &layout.title_identity),
            GraphicsHandle::layout_text(&candidate.identity, &layout.subtitle_identity),
        )
    } else {
        let axes = &mut candidate.axes;
        if let Some((string, lines)) = title {
            axes.title = string;
            axes.extras.label_lines[0] = lines;
            axes.extras.label_interpreters[0] = interpreter;
            axes.extras.title_style.font_size = font_size.or(axes.extras.title_style.font_size);
            axes.extras.title_style.bold = bold.or(axes.extras.title_style.bold);
            axes.extras.title_style.italic = italic.or(axes.extras.title_style.italic);
            if let Some(rgb) = color {
                axes.colors.get_or_insert_with(Default::default).title = Some(rgb);
            }
        }
        let text_handle = |identity| {
            GraphicsHandle::axes_object(
                &candidate.identity,
                &axes.identity,
                identity,
                GraphicsKind::Text,
            )
        };
        (
            text_handle(&axes.extras.title_identity),
            text_handle(&axes.extras.subtitle_identity),
        )
    };
    candidate.validate()?;
    let mut result = Vec::with_capacity(outputs);
    if is_title && outputs > 0 {
        result.push(Value::Container(
            GraphicsArray::scalar(title_handle, cancel)?.into(),
        ));
    }
    if result.len() < outputs {
        result.push(Value::Container(
            GraphicsArray::scalar(subtitle_handle, cancel)?.into(),
        ));
    }
    #[cfg(test)]
    tests::before_property_publish(cancel);
    engine.check_cancel()?;
    if target.is_some() && !is_layout {
        if super::ui_components::is_uiaxes_fig_id(id) {
            engine.publish_graphics_figure(id, candidate);
            return Ok(result);
        }
        engine.select_figure(id)?;
    }
    if is_layout {
        engine.publish_graphics_figure(id, candidate);
    } else {
        engine.figure = Some(candidate);
    }
    Ok(result)
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
    /// `movegui(h)` / `movegui(h, position)`: validates a live figure or
    /// figure descendant and the documented position argument. MatRust owns no
    /// host window placement, so a valid call leaves every figure, the current
    /// selection and the workspace unchanged.
    fn movegui_call(&self, args: &[Value]) -> Result<Value> {
        let (target, position) = match args {
            [target] => (target, None),
            [target, position] => (target, Some(position)),
            _ => return Err("Use movegui(h) or movegui(h, position)".into()),
        };
        let h = match target {
            Value::Container(Array::Graphics(a)) if a.len() == 1 => a.scalar_handle()?,
            _ => return Err("movegui requires one typed figure or graphics handle".into()),
        };
        if h.kind() == GraphicsKind::Root {
            return Err("movegui requires a figure or an object inside a figure".into());
        }
        self.locate_graphics(h)
            .map_err(|_| "movegui requires a live graphics handle")?;
        match position {
            None => {}
            Some(Value::Text(place)) => {
                if !matches!(
                    place.as_str(),
                    "north"
                        | "south"
                        | "east"
                        | "west"
                        | "northeast"
                        | "northwest"
                        | "southeast"
                        | "southwest"
                        | "center"
                        | "onscreen"
                ) {
                    return Err(format!("movegui: unknown position '{place}'"));
                }
            }
            Some(Value::Matrix(m)) if m.len() == 2 && m.data().iter().all(|v| v.is_finite()) => {}
            Some(_) => {
                return Err(
                    "movegui position must be a named position or a finite [x y] offset".into(),
                );
            }
        }
        Ok(Value::Void)
    }
    pub(in crate::engine) fn locate_graphics(&self, h: &GraphicsHandle) -> Result<Located> {
        if h.kind == GraphicsKind::Placeholder {
            return Err("Graphics placeholder is not a live handle".into());
        }
        if h.kind == GraphicsKind::Root {
            return Ok(Located {
                figure: 0,
                position: None,
                series: None,
                textbox: None,
                annotation: None,
            });
        }
        if h.kind == GraphicsKind::UiComponent {
            if super::ui_components::is_live(self, &h.object) {
                return Ok(Located {
                    figure: 0,
                    position: None,
                    series: None,
                    textbox: None,
                    annotation: None,
                });
            }
            return Err("Invalid or deleted object.".into());
        }
        let mut found = None;
        // Recheck structural limits because the Rust model fields are public.
        // Never traverse arbitrary externally supplied vectors before bounding
        // them, and never inspect samples to resolve a metadata identity.
        if self.other_figures.len() + usize::from(self.figure.is_some()) > 16 {
            return Err("Graphics lookup exceeds 16 figures".into());
        }
        if let Some(id) = self.dialogs.headless_id(h) {
            self.check_cancel()?;
            return Ok(Located {
                figure: id,
                position: None,
                series: None,
                textbox: None,
                annotation: None,
            });
        }
        for (id, f) in graphics_figures(self) {
            self.check_cancel()?;
            if f.layout.as_ref().is_some_and(|l| l.others.len() > 15) {
                return Err("Graphics lookup exceeds 16 axes".into());
            }
            if f.textboxes.len() > crate::plot::MAX_TEXTBOXES {
                return Err("Graphics lookup exceeds 64 textboxes".into());
            }
            if f.annotations.len() > crate::plot::MAX_ANNOTATIONS {
                return Err("Graphics lookup exceeds 64 annotations".into());
            }
            let mut series_count = 0usize;
            for (_, axes) in f.all_axes() {
                if axes.series.len() > crate::plot::MAX_PLOT_SERIES {
                    return Err("Graphics lookup exceeds series storage budget".into());
                }
                series_count += axes.series.len();
            }
            if series_count > crate::plot::MAX_PLOT_SERIES {
                return Err("Graphics lookup exceeds figure series storage budget".into());
            }
            if !h.kind.has_mutable_parent() && !f.identity.same_object(&h.figure) {
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
                    annotation: None,
                });
                continue;
            }
            if h.kind == GraphicsKind::TiledLayout
                && f.layout
                    .as_ref()
                    .is_some_and(|l| l.tiled.is_some() && l.identity.same_object(&h.object))
            {
                if found.is_some() {
                    return Err("Ambiguous duplicate graphics identity".into());
                }
                found = Some(Located {
                    figure: id,
                    position: None,
                    series: None,
                    textbox: None,
                    annotation: None,
                });
                continue;
            }
            if h.kind == GraphicsKind::Text
                && h.axes.is_none()
                && f.layout.as_ref().is_some_and(|l| {
                    l.title_identity.same_object(&h.object)
                        || l.subtitle_identity.same_object(&h.object)
                        || l.xlabel_identity.same_object(&h.object)
                        || l.ylabel_identity.same_object(&h.object)
                })
            {
                if found.is_some() {
                    return Err("Ambiguous duplicate graphics identity".into());
                }
                found = Some(Located {
                    figure: id,
                    position: None,
                    series: None,
                    textbox: None,
                    annotation: None,
                });
                continue;
            }
            if matches!(
                h.kind,
                GraphicsKind::SliceViewer | GraphicsKind::OrthosliceViewer
            ) {
                if h.is_live_in(f) {
                    if found.is_some() {
                        return Err("Ambiguous duplicate graphics identity".into());
                    }
                    found = Some(Located {
                        figure: id,
                        position: None,
                        series: None,
                        textbox: None,
                        annotation: None,
                    });
                }
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
                            annotation: None,
                        });
                    }
                }
                continue;
            }
            if h.kind == GraphicsKind::Annotation {
                for (index, ann) in f.annotations.iter().enumerate() {
                    if h.object.same_object(&ann.identity) {
                        if found.is_some() {
                            return Err("Ambiguous duplicate graphics identity".into());
                        }
                        found = Some(Located {
                            figure: id,
                            position: None,
                            series: None,
                            textbox: None,
                            annotation: Some(index),
                        });
                    }
                }
                continue;
            }
            for (position, a) in f.all_axes() {
                if h.kind != GraphicsKind::Volume
                    && !h
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
                        annotation: None,
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
                        annotation: None,
                    });
                }
                let owned = match h.kind {
                    GraphicsKind::Volume => {
                        if a.extras.volumes.len() > crate::plot::MAX_VOLUMES {
                            return Err("Graphics lookup exceeds 16 volumes per axes".into());
                        }
                        a.extras
                            .volumes
                            .iter()
                            .any(|v| h.object.same_object(&v.identity))
                    }
                    GraphicsKind::Text
                    | GraphicsKind::Colorbar
                    | GraphicsKind::Image
                    | GraphicsKind::NumericRuler
                    | GraphicsKind::ConstantLine
                    | GraphicsKind::Rectangle => a.extras.owns(&h.object),
                    _ => false,
                };
                if owned {
                    if found.is_some() {
                        return Err("Ambiguous duplicate graphics identity".into());
                    }
                    found = Some(Located {
                        figure: id,
                        position: Some(position),
                        series: None,
                        textbox: None,
                        annotation: None,
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
                        annotation: None,
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
                                annotation: None,
                            });
                        }
                    }
                }
            }
        }
        found.ok_or("Graphics handle is stale or does not belong to this engine graph".into())
    }
    pub(in crate::engine) fn graphics_figure(&self, id: u32) -> Result<&Figure> {
        if super::ui_components::is_uiaxes_fig_id(id) {
            return self
                .ui
                .uiaxes_figures
                .get(&id)
                .ok_or("Graphics figure is no longer live".into());
        }
        self.figures()
            .find(|(n, _)| *n == id)
            .map(|(_, f)| f)
            .ok_or("Graphics figure is no longer live".into())
    }
    pub(in crate::engine) fn publish_graphics_figure(&mut self, id: u32, mut figure: Figure) {
        // Colormap, CLim or background changes recolour the surfaces; their
        // geometry and face counts are unchanged, so this cannot fail on
        // admission and a failure keeps the previous colours.
        if self.surface_records() || surface_commands::has_indexed_colors(&figure) {
            let mut recoloured = figure.clone();
            if self.surface_refresh(&mut recoloured, false).is_ok() {
                figure = recoloured;
            }
        }
        if super::ui_components::is_uiaxes_fig_id(id) {
            self.ui.uiaxes_figures.insert(id, figure);
            return;
        }
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
        let (title, lines) = super::graphics_text_lines(title, &self.cancel)?;
        let mut interpreter = crate::plot::text_markup::Interpreter::Tex;
        if !properties.len().is_multiple_of(2) {
            return Err("sgtitle properties require name/value pairs".into());
        }
        for pair in properties.as_chunks::<2>().0 {
            if !text(&pair[0])?.eq_ignore_ascii_case("Interpreter") {
                return Err("Unsupported sgtitle property".into());
            }
            interpreter = crate::plot::text_markup::Interpreter::parse(text(&pair[1])?)?;
        }
        let mut candidate = if id == self.current_figure {
            self.figure.clone().unwrap_or_default()
        } else {
            self.graphics_figure(id)?.clone()
        };
        candidate.set_shared_title(title)?;
        let layout = candidate.layout.as_mut().unwrap();
        layout.text_lines[0] = lines;
        layout.text_interpreters[0] = interpreter;
        let result = GraphicsArray::scalar(
            GraphicsHandle::layout_text(&candidate.identity, &layout.title_identity),
            &self.cancel,
        )?;
        candidate.validate()?;
        #[cfg(test)]
        tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(id, candidate);
        Ok(Value::Container(result.into()))
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
        if name == "double"
            && let Some(Value::Container(Array::Graphics(handles))) = args.first()
        {
            if args.len() != 1 {
                return Err("double requires one argument".into());
            }
            self.preflight_elements(handles.len())?;
            return Ok(Some(Value::Matrix(handles.to_double(&self.cancel)?)));
        }
        if matches!(name, "properties" | "fieldnames")
            && let [Value::Container(Array::Graphics(handles))] = args
            && handles.len() == 1
            && handles.scalar_handle()?.kind() == GraphicsKind::Root
        {
            let names = dot_syntax::ROOT_PROPERTIES
                .iter()
                .map(|name| {
                    Ok(Arc::new(
                        crate::CharacterRow::from_text(name, &self.cancel)?.into(),
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            return Ok(Some(Value::Container(
                crate::CellArray::from_shared(names.len(), 1, names, &self.cancel)?.into(),
            )));
        }
        if let Some(value) = self.gobj_call(name, args, out)? {
            return Ok(Some(value));
        }
        if matches!(name, "title" | "subtitle") {
            return heading_call(self, name, args, 1).map(|mut values| values.pop());
        }
        let has_leading_handle = matches!(args.first(), Some(Value::Container(Array::Graphics(_))));
        if !matches!(
            name,
            "figure"
                | "movegui"
                | "gcf"
                | "gca"
                | "groot"
                | "axes"
                | "get"
                | "set"
                | "reset"
                | "isgraphics"
                | "ishghandle"
                | "ishold"
                | "findobj"
                | "findall"
                | "allchild"
                | "patch"
                | "xline"
                | "yline"
                | "rectangle"
                | "annotation"
                | "sgtitle"
                | "legend"
        ) && !has_leading_handle
        {
            return Ok(None);
        }
        self.check_cancel()?;
        if matches!(name, "get" | "set")
            && let Some(Value::Container(Array::Graphics(arr))) = args.first()
            && arr.len() == 1
            && arr.scalar_handle()?.kind() == GraphicsKind::UiComponent
        {
            let h = arr.scalar_handle()?.clone();
            return if name == "get" {
                super::ui_components::get_property(self, &h, &args[1..]).map(Some)
            } else {
                super::ui_components::set_properties(self, &h, &args[1..])?;
                Ok(Some(Value::Void))
            };
        }
        if name == "close"
            && let Some(Value::Container(Array::Graphics(arr))) = args.first()
            && arr.len() == 1
            && arr.scalar_handle()?.kind() == GraphicsKind::UiComponent
        {
            if args.len() > 2 {
                return Err("close requires one figure handle".into());
            }
            let force = match args.get(1) {
                None => false,
                Some(Value::Text(s) | Value::QuotedString(s))
                    if s.eq_ignore_ascii_case("force") =>
                {
                    true
                }
                Some(Value::Container(Array::Character(c)))
                    if c.is_text_row()
                        && c.to_utf8(&self.cancel)
                            .is_ok_and(|s| s.eq_ignore_ascii_case("force")) =>
                {
                    true
                }
                _ => return Err("close: the second argument must be 'force'".into()),
            };
            let target = arr.scalar_handle()?.clone();
            super::ui_components::close_figure(self, &target, force, out)?;
            return Ok(Some(Value::Void));
        }
        let mut normalized = self.text_arguments(args.to_vec())?;
        if matches!(name, "xlabel" | "ylabel" | "zlabel" | "sgtitle") {
            let index = usize::from(has_leading_handle);
            if matches!(args.get(index), Some(Value::QuotedString(_))) {
                normalized[index] = args[index].clone();
            }
        }
        if name == "set" && !args.is_empty() {
            preserve_string_property_values(&args[1..], &mut normalized[1..]);
        }
        let args = normalized;
        if name == "movegui" {
            return self.movegui_call(&args).map(Some);
        }
        if name == "ishold" {
            let held = match args.as_slice() {
                [] => self.figure.as_ref().is_some_and(|figure| figure.hold),
                [target] => {
                    let h = handle(target)?;
                    if h.kind() != GraphicsKind::Axes {
                        return Err("ishold requires a live axes handle".into());
                    }
                    let target = self.locate_graphics(h)?;
                    self.graphics_figure(target.figure)?
                        .all_axes()
                        .any(|(position, axes)| Some(position) == target.position && axes.hold)
                }
                _ => return Err("Use ishold or ishold(ax)".into()),
            };
            return Ok(Some(Value::Logical(LogicalMatrix::scalar(held))));
        }
        if name == "findobj" {
            return find_objects::find_objects(self, &args).map(Some);
        }
        if name == "findall" {
            return find_objects::find_all(self, &args).map(Some);
        }
        if name == "allchild" {
            return find_objects::all_children(self, &args).map(Some);
        }
        if name == "sgtitle" {
            return self.shared_title_graphics(&args).map(Some);
        }
        if name == "legend" {
            return self.legend_object_call(&args, out).map(Some);
        }
        if name == "patch" {
            return self.create_patch_object(&args).map(Some);
        }
        if name == "xline" {
            return self
                .create_constant_line_object(crate::plot::InterceptAxis::X, &args)
                .map(Some);
        }
        if name == "yline" {
            return self
                .create_constant_line_object(crate::plot::InterceptAxis::Y, &args)
                .map(Some);
        }
        if name == "rectangle" {
            return self.create_rectangle_object(&args).map(Some);
        }
        if name == "annotation" {
            return self.create_annotation_object(&args).map(Some);
        }
        if name == "groot" {
            if !args.is_empty() {
                return Err("groot takes no arguments".into());
            }
            return Ok(Some(Value::Container(
                GraphicsArray::scalar(GraphicsHandle::root(), &self.cancel)?.into(),
            )));
        }
        if name == "reset" {
            let [target] = args.as_slice() else {
                return Err("reset requires one graphics handle".into());
            };
            let h = handle_or_root(target)?;
            if h.kind() != GraphicsKind::Root {
                return Err("reset currently supports groot / 0".into());
            }
            self.reset_root();
            return Ok(Some(Value::Void));
        }
        if matches!(name, "isgraphics" | "ishghandle") {
            if args.len() != 1 {
                return Err(format!("{name} initially accepts one handle array"));
            }
            return match &args[0] {
                Value::Container(Array::Graphics(h)) => {
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
                    Ok(Some(Value::Logical(LogicalMatrix::from_column_vec(
                        h.rows(),
                        h.cols(),
                        values,
                        &self.cancel,
                    )?)))
                }
                Value::Matrix(m) => {
                    let ids = self.figure_ids();
                    let values = m
                        .data()
                        .iter()
                        .map(|&v| {
                            u8::from(
                                v == 0.0
                                    || (name == "ishghandle"
                                        && v.is_finite()
                                        && v.fract() == 0.
                                        && v >= 1.
                                        && v <= f64::from(u32::MAX)
                                        && (ids.contains(&(v as u32))
                                            || self
                                                .dlg_handle(&Value::Matrix(Matrix::scalar(v)))
                                                .is_some())),
                            )
                        })
                        .collect();
                    Ok(Some(Value::Logical(LogicalMatrix::from_column_vec(
                        m.rows(),
                        m.cols(),
                        values,
                        &self.cancel,
                    )?)))
                }
                _ => Err(format!(
                    "{name} currently requires a typed handle array or numeric 0"
                )),
            };
        }
        if matches!(name, "figure" | "gcf" | "gca") {
            if name != "figure" && !args.is_empty() {
                return Err(format!("{name} takes no arguments"));
            }
            // `figure 3`: command syntax passes the char word '3' (fix line 12).
            let word = match args.as_slice() {
                [one] if name == "figure" => super::graphics::number_word(one),
                _ => None,
            };
            let args = match word {
                Some(n) => vec![Value::Matrix(Matrix::scalar(n))],
                None => args.to_vec(),
            };
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
            let existing = self
                .figures()
                .find(|(number, _)| *number == id)
                .map(|(_, f)| f.clone());
            let mut figure = match existing {
                Some(f) => f,
                None => {
                    let mut f = Figure::default();
                    self.apply_figure_defaults(&mut f)?;
                    self.graphics_new_axes_defaults(&mut f, name != "gca")?;
                    f
                }
            };
            for pair in properties.as_chunks::<2>().0 {
                if text(&pair[0])?.eq_ignore_ascii_case("Visible") {
                    let vis = match text(&pair[1])?.to_ascii_lowercase().as_str() {
                        "on" => true,
                        "off" => false,
                        _ => return Err("Visible supports on or off".into()),
                    };
                    figure.visible = vis;
                    self.set_figure_visible(&figure.identity, vis);
                } else {
                    figure_appearance(&mut figure, pair, self.graphics_theme_context)?;
                }
            }
            let axes_defaults = if name == "gca" {
                figure.activate_tile();
                self.graphics_existing_axes_defaults(&mut figure, None)?
            } else {
                None
            };
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
            self.graphics_finish_axes_defaults(&figure, axes_defaults);
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
            let mut hold: Option<bool> = None;
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
                        hold = Some(match text(&pair[1])? {
                            "add" => true,
                            "replace" => false,
                            _ => return Err("Axes NextPlot supports add or replace".into()),
                        })
                    }
                    _ => return Err(
                        "Axes constructor supports Parent, normalized Units, Position and NextPlot"
                            .into(),
                    ),
                }
            }
            let position = Position::from_normalized(position)?;
            let existing = self
                .figures()
                .find(|(id, _)| *id == parent)
                .map(|(_, f)| f.clone());
            let mut figure = match existing {
                Some(f) => f,
                None => {
                    let mut f = Figure::default();
                    self.apply_figure_defaults(&mut f)?;
                    f
                }
            };
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
            self.apply_axes_defaults(&mut figure)?;
            if let Some(hold) = hold {
                figure.axes.hold = hold;
            }
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
            // Numeric matrices stay on the scalar path, which rejects them.
            // Only an empty or multi-element typed handle array is a bulk set.
            if name == "set"
                && let Some(handles) = match args.first() {
                    Some(Value::Container(Array::Graphics(handles))) if handles.len() != 1 => {
                        Some(handles.clone())
                    }
                    _ => None,
                }
            {
                self.set_many_graphics_properties(&handles, &args[1..])?;
                return Ok(Some(Value::Void));
            }
            let target = args
                .first()
                .ok_or("Graphics property operation requires a handle")?;
            let h = handle_or_root(target)?;
            if h.kind() == GraphicsKind::Root {
                return if name == "get" {
                    self.get_root_property(&args[1..]).map(Some)
                } else {
                    self.set_root_properties(&args[1..])?;
                    Ok(Some(Value::Void))
                };
            }
            let located = self.locate_graphics(&h)?;
            return if name == "get" {
                self.get_graphics_property(&h, located, &args[1..])
                    .map(Some)
            } else {
                self.set_graphics_properties(&h, located, &args[1..])?;
                Ok(Some(Value::Void))
            };
        }
        if name == "close" && has_leading_handle {
            if args.len() > 2 || handle(&args[0])?.kind() != GraphicsKind::Figure {
                return Err("close requires one figure handle".into());
            }
            let target = handle(&args[0])?;
            if self.dialogs.headless_id(target).is_some() {
                return self.graphics("close", args, out).map(Some);
            }
            let id = self.locate_graphics(target)?.figure;
            // `close(h,'force')`: graphics() checks the option.
            let mut forwarded = vec![Value::Matrix(Matrix::scalar(f64::from(id)))];
            forwarded.extend(args.get(1).cloned());
            return self.graphics("close", forwarded, out).map(Some);
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
                    | "xlabel"
                    | "ylabel"
                    | "hold"
                    | "grid"
                    | "xlim"
                    | "ylim"
                    | "axis"
                    | "xticks"
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
                    | "plot3"
                    | "scatter3"
                    | "zlabel"
                    | "zlim"
                    | "view"
            )
        {
            let h = handle(&args[0])?;
            if h.kind() == GraphicsKind::TiledLayout && matches!(name, "xlabel" | "ylabel") {
                let located = self.locate_graphics(h)?;
                let tail = &args[1..];
                if tail.is_empty() || tail.len() > 11 || tail.len().is_multiple_of(2) {
                    return Err(format!(
                        "{name} expects a label and optional Name,Value pairs after layout handle"
                    ));
                }
                let mut interpreter = crate::plot::text_markup::Interpreter::Tex;
                let mut style = crate::plot::LabelStyle::default();
                for pair in tail[1..].as_chunks::<2>().0 {
                    match text(&pair[0])?.to_ascii_lowercase().as_str() {
                        "interpreter" => {
                            interpreter =
                                crate::plot::text_markup::Interpreter::parse(text(&pair[1])?)?;
                        }
                        "fontsize" => {
                            let size = dot_syntax::real_scalar(&pair[1], "FontSize")?;
                            crate::plot::validate_font_size(size)?;
                            style.font_size = Some(size);
                        }
                        "fontweight" => style.bold = Some(dot_syntax::text_font_weight(&pair[1])?),
                        "fontangle" => style.italic = Some(dot_syntax::text_font_angle(&pair[1])?),
                        _ => return Err("Unsupported layout text property".into()),
                    }
                }
                let (label, lines) = super::graphics_text_lines(&tail[0], &self.cancel)?;
                let mut selected = self.graphics_figure(located.figure)?.clone();
                let layout = selected
                    .layout
                    .as_mut()
                    .ok_or("TiledChartLayout is no longer live")?;
                let index = if name == "xlabel" { 1 } else { 2 };
                layout.text_lines[index] = lines;
                layout.text_interpreters[index] = interpreter;
                let target_style = &mut layout.label_styles[index - 1];
                target_style.font_size = style.font_size.or(target_style.font_size);
                target_style.bold = style.bold.or(target_style.bold);
                target_style.italic = style.italic.or(target_style.italic);
                let identity = match name {
                    "xlabel" => {
                        layout.xlabel = label;
                        &layout.xlabel_identity
                    }
                    _ => {
                        layout.ylabel = label;
                        &layout.ylabel_identity
                    }
                };
                let result = GraphicsArray::scalar(
                    GraphicsHandle::layout_text(&selected.identity, identity),
                    &self.cancel,
                )?;
                selected.validate()?;
                self.check_cancel()?;
                self.publish_graphics_figure(located.figure, selected);
                return Ok(Some(Value::Container(result.into())));
            }
            if h.kind() != GraphicsKind::Axes {
                return Err("Leading graphics target must be an axes handle".into());
            }
            let tail = args[1..].to_vec();
            return self.call_on_axes(h, name, tail, out).map(Some);
        }
        Ok(None)
    }

    /// Runs `name(tail)` with axes `h` current; see [`Self::with_axes`].
    pub(in crate::engine) fn call_on_axes(
        &mut self,
        h: &GraphicsHandle,
        name: &str,
        tail: Vec<Value>,
        out: &mut Execution,
    ) -> Result<Value> {
        self.with_axes(h, |e| e.call(name, tail, out))
    }

    /// Runs `f` with axes `h` current. A uiaxes is published back to its host
    /// and the classic current figure stays as it was.
    pub(in crate::engine) fn with_axes<T>(
        &mut self,
        h: &GraphicsHandle,
        f: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let located = self.locate_graphics(h)?;
        let mut selected = self.graphics_figure(located.figure)?.clone();
        select_live_axes(&mut selected, located.position.unwrap())?;
        let is_uiaxes = super::ui_components::is_uiaxes_fig_id(located.figure);
        // Roll back metadata/current selection on any downstream validation,
        // allocation or Stop error. Sample buffers remain immutable/shared.
        let old = (
            self.current_figure,
            self.figure.clone(),
            self.other_figures.clone(),
        );
        let prev_held = self.ui.held_classic.take();
        let result = (|| {
            if is_uiaxes {
                self.ui.held_classic = self.figure.take().map(|f| (self.current_figure, f));
            } else {
                self.select_figure(located.figure)?;
            }
            self.figure = Some(selected);
            let value = f(self)?;
            self.check_cancel()?;
            Ok(value)
        })();
        self.ui.held_classic = prev_held;
        if is_uiaxes {
            if result.is_ok()
                && let Some(updated) = self.figure.take()
            {
                self.publish_graphics_figure(located.figure, updated);
            }
            (self.current_figure, self.figure, self.other_figures) = old;
        } else if result.is_err() {
            (self.current_figure, self.figure, self.other_figures) = old;
        }
        result
    }

    fn get_graphics_property(
        &mut self,
        h: &GraphicsHandle,
        located: Located,
        args: &[Value],
    ) -> Result<Value> {
        if args.len() != 1 {
            return Err("get initially requires one explicit property name".into());
        }
        if h.kind() == GraphicsKind::Axes
            && self.ui.is_uiaxes(&h.object)
            && let Ok(prop_name) = text(&args[0])
            && super::ui_components::is_uiaxes_ui_property(prop_name)
        {
            return super::ui_components::get_property(self, h, args);
        }
        if let Some(property) =
            self.dialogs
                .headless_property(located.figure, &args[0], &self.cancel)
        {
            return property;
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
        if let Some(index) = located.annotation {
            return self.get_annotation_property(located.figure, index, args);
        }
        if let Some(value) = self.surface_get(h, text(&args[0])?)? {
            return Ok(value);
        }
        // Chart objects (histogram, errorbar, ...) and polar axes answer first.
        if matches!(h.kind(), GraphicsKind::Series | GraphicsKind::Axes)
            && let Some(value) = self.p2_get(h, located.figure, text(&args[0])?)?
        {
            return Ok(value);
        }
        if let Some(value) = self.gobj_get(h, located, &args[0])? {
            return Ok(value);
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
            if let Some((super::graphics_root::DefaultQueryKind::Default, obj_type, canon)) =
                super::graphics_root::parse_default_or_factory(text(&args[0])?)?
            {
                if let Some(val) = self
                    .figure_defaults
                    .get(&figure.identity.recovery_key())
                    .and_then(|m| m.get(&(obj_type, canon)))
                    .or_else(|| self.root.defaults.get(&(obj_type, canon)))
                {
                    return Ok(val.value.clone());
                }
                return super::graphics_root::factory_value(obj_type, canon);
            }
            return match name.as_str() {
                "name" => string(&figure.name),
                "numbertitle" => string(if figure.number_title { "on" } else { "off" }),
                "visible" => string(if figure.visible && !self.is_figure_hidden(&figure.identity) {
                    "on"
                } else {
                    "off"
                }),
                "position" | "innerposition" | "outerposition" => Ok(Value::Matrix(
                    Matrix::from_column_slice(1, 4, &figure.position_in_units())?,
                )),
                "units" => string(figure.units.as_str()),
                "resize" => string(if figure.resize { "on" } else { "off" }),
                "windowstate" => string(figure.window_state.as_str()),
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
                "currentaxes" => {
                    let result = GraphicsArray::scalar(
                        GraphicsHandle::axes(&figure.identity, &figure.axes.identity),
                        &self.cancel,
                    )?;
                    if self.graphics_axes_defaults_pending(figure) {
                        let mut figure = figure.clone();
                        let values = self.graphics_existing_axes_defaults(&mut figure, None)?;
                        figure.validate()?;
                        self.check_cancel()?;
                        self.graphics_finish_axes_defaults(&figure, values);
                        self.publish_graphics_figure(located.figure, figure);
                    }
                    Ok(Value::Container(result.into()))
                }
                _ => Err(
                    "Supported figure queries: Type, Number, CurrentAxes, Name, NumberTitle, Visible, Position, InnerPosition, OuterPosition, Units, Resize, WindowState, Color, Theme, ThemeMode, ColorMode and InvertHardcopy".into(),
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
            if s.indexed_patch.is_some() && name == "edgecolor" && s.face_edge_colors.is_some() {
                return string("flat");
            }
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
            if let Some(value) = object_commands::indexed_patch_property(s, &name)? {
                return Ok(value);
            }
            return match name.as_str() {
                "type" if s.kind==crate::plot::PlotKind::Line=>string("line"),
                "type" if s.kind==crate::plot::PlotKind::Patch=>string("patch"),
                "type" if s.kind==crate::plot::PlotKind::Bar=>string("bar"),
                "type" if s.kind==crate::plot::PlotKind::Scatter=>string("scatter"),
                "type" if s.kind==crate::plot::PlotKind::Stairs=>string("stair"),
                "type" if s.kind==crate::plot::PlotKind::Stem=>string("stem"),
                "barlayout" if s.kind==crate::plot::PlotKind::Bar=>string(match s.bar_layout {crate::plot::BarLayout::Grouped=>"grouped",crate::plot::BarLayout::Stacked=>"stacked"}),
                "barwidth" if s.kind==crate::plot::PlotKind::Bar=>Ok(Value::Matrix(Matrix::scalar(s.bar_relative_width.unwrap_or(0.8)))),
                "facecolor" if s.kind==crate::plot::PlotKind::Bar=>match s.face_color_mode {
                    crate::plot::FaceColorMode::Flat=>string("flat"),
                    crate::plot::FaceColorMode::None=>string("none"),
                    crate::plot::FaceColorMode::Color=>Ok(Value::Matrix(Matrix::from_column_slice(1,3,&figure.resolve_appearance(self.graphics_theme_context).series_color(s).map(|c|f64::from(c)/255.))?)),
                },
                "facecolor" if s.kind==crate::plot::PlotKind::Patch=>if s.face_color_none { string("none") } else if s.face_color_mode == crate::plot::FaceColorMode::Flat { string(if s.patch_vertex_cdata.is_some() { "interp" } else { "flat" }) } else { Ok(Value::Matrix(Matrix::from_column_slice(1,3,&figure.resolve_appearance(self.graphics_theme_context).series_color(s).map(|c|f64::from(c)/255.))?)) },
                "cdata" if s.kind==crate::plot::PlotKind::Patch && s.patch_vertex_cdata.is_some() => { let v=s.patch_vertex_cdata.as_ref().unwrap(); Ok(Value::Matrix(Matrix::from_column_slice(v.source.len(),1,&v.source)?)) },
                "cdata" if s.kind==crate::plot::PlotKind::Patch && s.patch_cdata.is_some() => Ok(Value::Matrix(Matrix::scalar(s.patch_cdata.unwrap()))),
                "cdatamapping" if s.kind==crate::plot::PlotKind::Patch => string("scaled"),
                "facealpha" if s.kind==crate::plot::PlotKind::Bar=>Ok(Value::Matrix(Matrix::scalar(s.face_alpha.unwrap_or(1.)))),
                "cdata" if s.kind==crate::plot::PlotKind::Bar=>{
                    let n=s.points.len();
                    let default_rgb=figure.resolve_appearance(self.graphics_theme_context).series_color(s).map(|c|f64::from(c)/255.);
                    let mut data=Vec::with_capacity(n*3);
                    for col in 0..3 {
                        if let Some(colors)=&s.face_colors {
                            data.extend(colors.iter().map(|rgb|f64::from(rgb[col])/255.));
                        } else {
                            data.extend(std::iter::repeat_n(default_rgb[col],n));
                        }
                    }
                    Ok(Value::Matrix(Matrix::from_column_vec(n,3,data)?))
                }
                "edgecolor" if matches!(s.kind,crate::plot::PlotKind::Patch | crate::plot::PlotKind::Bar)=>match s.face_edge_color {None=>string("none"),Some(rgb)=>Ok(Value::Matrix(Matrix::from_column_slice(1,3,&rgb.map(|c|f64::from(c)/255.))?))},
                "facealpha" if s.kind == crate::plot::PlotKind::Patch => Ok(Value::Matrix(Matrix::scalar(s.face_alpha.unwrap_or(1.)))),
                "edgealpha" if s.kind == crate::plot::PlotKind::Patch => Ok(Value::Matrix(Matrix::scalar(s.edge_alpha.unwrap_or(1.)))),
                "linestyle"=>string(match s.pattern {crate::plot::LinePattern::Solid=>"-",crate::plot::LinePattern::Dashed=>"--",crate::plot::LinePattern::Dotted=>":",crate::plot::LinePattern::DashDot=>"-.",crate::plot::LinePattern::None=>"none"}),
                "displayname"=>string(&s.label),"visible"=>string(if s.visible {"on"} else {"off"}),
                "linewidth" if matches!(s.kind,crate::plot::PlotKind::Bar|crate::plot::PlotKind::Patch|crate::plot::PlotKind::Scatter)=>Ok(Value::Matrix(Matrix::scalar(s.line_width.unwrap_or(0.5)))),
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
            "xtick" if axes.extras.leaf_ticks.is_some() => {
                let n = axes.extras.leaf_ticks.as_ref().unwrap().len();
                Ok(Value::Matrix(Matrix::from_column_vec(
                    1,
                    n,
                    (1..=n).map(|i| i as f64).collect(),
                )?))
            }
            "xticklabel" if axes.extras.leaf_ticks.is_some() => {
                let labels = axes.extras.leaf_ticks.as_ref().unwrap();
                let slots = labels
                    .iter()
                    .map(|s| {
                        crate::CharacterRow::from_text(s, &self.cancel)
                            .map(|c| std::sync::Arc::new(Array::Character(c)))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(Value::Container(Array::Cell(
                    crate::CellArray::from_shared(labels.len(), 1, slots, &self.cancel)?,
                )))
            }
            "xlim" | "ylim" => {
                let (x, y) = axes.bounds();
                let axis = usize::from(name == "ylim");
                super::plot_time_axes::ruler_values_to_value(
                    &axes.extras.rulers[axis].kind,
                    if axis == 0 { &x } else { &y },
                    &self.cancel,
                )
            }
            _ => Err("Unsupported axes property query".into()),
        }
    }
    pub(super) fn graphics_apply_axes_pairs(
        &self,
        figure: &mut Figure,
        args: &[Value],
    ) -> Result<(bool, bool)> {
        if args.is_empty() {
            return Ok((false, false));
        }
        let mut sync_x = false;
        let mut sync_y = false;
        let mut placement = figure.initial_axes_placement()?;
        let mut change_placement = false;
        let mut modern = None;
        let mut legacy = None;
        for pair in args.as_chunks::<2>().0 {
            match text(&pair[0])?.to_ascii_lowercase().as_str() {
                "color" | "xcolor" | "ycolor" | "gridcolor" | "gridalpha" | "xcolormode"
                | "ycolormode" | "gridcolormode" | "gridalphamode" => {
                    set_axes_color(figure, pair, self.graphics_theme_context, &self.cancel)?;
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
                                "Grid visibility requires on, off, logical or numeric 0/1".into()
                            );
                        }
                    };
                    if text(&pair[0])?.eq_ignore_ascii_case("XGrid") {
                        figure.axes.xgrid = enabled;
                    } else {
                        figure.axes.ygrid = enabled;
                    }
                }
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
                    let is_x = text(&pair[0])?.eq_ignore_ascii_case("XLim");
                    let axis = usize::from(!is_x);
                    let vals = super::plot_time_axes::parse_ruler_values(
                        &figure.axes.extras.rulers[axis].kind,
                        &pair[1],
                        "Limits",
                        &self.cancel,
                    )?;
                    if vals.len() != 2 {
                        return Err("Limits must be a two-element vector".into());
                    }
                    let values = [vals[0], vals[1]];
                    crate::plot::validate_axis_limits(values)?;
                    let stored = if values == [f64::NEG_INFINITY, f64::INFINITY] {
                        None
                    } else {
                        Some(values)
                    };
                    if is_x {
                        figure.axes.xlim = stored;
                        sync_x = true;
                    } else {
                        figure.axes.ylim = stored;
                        sync_y = true;
                    }
                    let axes_id = figure.axes.identity.clone();
                    figure.sync_linked_axes(&axes_id, is_x, !is_x);
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
                    let value = text(&pair[1])?;
                    legacy = Some(if value.eq_ignore_ascii_case("position") {
                        PositionConstraint::InnerPosition
                    } else if value.eq_ignore_ascii_case("outerposition") {
                        PositionConstraint::OuterPosition
                    } else {
                        return Err(
                            "ActivePositionProperty must be position or outerposition".into()
                        );
                    });
                    change_placement = true;
                }
                _ => {
                    return Err(
                        "Unsupported axes property; no property transaction was published".into(),
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
        Ok((sync_x, sync_y))
    }
    /// Prepare the complete update on only the involved figures before publishing.
    /// Other record-backed charts and linked limits require a wider transaction
    /// and are explicitly unsupported; scalar setters keep their behavior.
    fn set_many_graphics_properties(
        &mut self,
        handles: &GraphicsArray,
        args: &[Value],
    ) -> Result<()> {
        if args.is_empty() || args.len() > 32 || !args.len().is_multiple_of(2) {
            return Err("set requires 1..16 property/value pairs".into());
        }
        if handles.is_empty() {
            for pair in args.as_chunks::<2>().0 {
                text(&pair[0])?;
            }
            return Ok(());
        }
        let mut targets = Vec::new();
        targets
            .try_reserve_exact(handles.len())
            .map_err(|_| "Cannot allocate graphics property targets")?;
        let mut staged = Engine {
            cancel: self.cancel.clone(),
            graphics_theme_context: self.graphics_theme_context,
            current_figure: self.current_figure,
            ..Engine::default()
        };
        for i in 0..handles.len() {
            self.check_cancel()?;
            let h = handles.get_linear(i)?.clone();
            if h.kind() == GraphicsKind::Root {
                return Err("This graphics operation requires one scalar handle".into());
            }
            let located = self.locate_graphics(&h)?;
            let linked_target = match h.kind() {
                GraphicsKind::Axes => Some(&h.object),
                GraphicsKind::NumericRuler => h.axes.as_ref(),
                _ => None,
            };
            let changes_limits = args.as_chunks::<2>().0.iter().any(|pair| {
                text(&pair[0]).is_ok_and(|name| {
                    ["xlim", "ylim", "limits", "limitsmode"]
                        .iter()
                        .any(|candidate| name.eq_ignore_ascii_case(candidate))
                })
            });
            let linked_limits = changes_limits
                && linked_target.is_some_and(|target| {
                    self.linked_axes
                        .iter()
                        .any(|link| link.axes.iter().any(|id| id.same_object(target)))
                });
            if self.p2_get(&h, located.figure, "type")?.is_some() || linked_limits {
                return Err(
                    "Bulk set for record-backed charts or linked axes is not supported yet".into(),
                );
            }
            // Only the plot model is staged. UI component properties also need
            // their owning node, which is outside this transaction.
            if self.ui.is_uiaxes(&h.object)
                && args.as_chunks::<2>().0.iter().any(|[key, _]| {
                    text(key).is_ok_and(super::ui_components::is_uiaxes_ui_property)
                })
            {
                return Err("Bulk set of UI component properties is not supported yet".into());
            }
            if staged.graphics_figure(located.figure).is_err() {
                let figure = self.graphics_figure(located.figure)?.clone();
                let key = figure.identity.recovery_key();
                if let Some(defaults) = self.figure_defaults.get(&key) {
                    staged.figure_defaults.insert(key, defaults.clone());
                }
                if self.hidden_figures.contains(&key) {
                    staged.hidden_figures.insert(key);
                }
                if super::ui_components::is_uiaxes_fig_id(located.figure) {
                    staged.ui.uiaxes_figures.insert(located.figure, figure);
                } else if located.figure == staged.current_figure {
                    staged.figure = Some(figure);
                } else {
                    staged.other_figures.insert(located.figure, figure);
                }
            }
            targets.push((h, located));
        }
        let figures: Vec<_> = graphics_figures(&staged)
            .map(|(_, figure)| figure)
            .collect();
        staged.surf = self.surf.for_figures(&figures);
        for (h, located) in &targets {
            staged.set_graphics_properties(h, *located, args)?;
        }
        self.check_cancel()?;
        // No fallible operations after this point. Only relevant figures and
        // their associated defaults and visibility are replaced.
        self.surf.publish_from(std::mem::take(&mut staged.surf));
        for (id, figure) in graphics_figures(&staged) {
            let key = figure.identity.recovery_key();
            match staged.figure_defaults.get(&key) {
                Some(defaults) => {
                    self.figure_defaults.insert(key, defaults.clone());
                }
                None => {
                    self.figure_defaults.remove(&key);
                }
            }
            if staged.hidden_figures.contains(&key) {
                self.hidden_figures.insert(key);
            } else {
                self.hidden_figures.remove(&key);
            }
            self.publish_graphics_figure(id, figure.clone());
        }
        Ok(())
    }
    pub(in crate::engine) fn set_graphics_properties(
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
        let uiaxes_remaining: Vec<Value>;
        let args = if h.kind() == GraphicsKind::Axes && self.ui.is_uiaxes(&h.object) {
            if !args.len().is_multiple_of(2) {
                return Err("set expects Name, Value property pairs.".into());
            }
            let mut ui_pairs = Vec::new();
            let mut axes_pairs = Vec::new();
            for pair in args.as_chunks::<2>().0 {
                if let Ok(prop_name) = text(&pair[0])
                    && super::ui_components::is_uiaxes_ui_property(prop_name)
                {
                    ui_pairs.push(pair[0].clone());
                    ui_pairs.push(pair[1].clone());
                } else {
                    axes_pairs.push(pair[0].clone());
                    axes_pairs.push(pair[1].clone());
                }
            }
            if !ui_pairs.is_empty() {
                super::ui_components::set_properties(self, h, &ui_pairs)?;
            }
            if axes_pairs.is_empty() {
                return Ok(());
            }
            uiaxes_remaining = axes_pairs;
            uiaxes_remaining.as_slice()
        } else {
            args
        };
        if h.kind() == GraphicsKind::Volume {
            return super::volume::show::volume_set(self, &h.object, located, args);
        }
        if matches!(
            h.kind(),
            GraphicsKind::SliceViewer | GraphicsKind::OrthosliceViewer
        ) {
            return super::volume::slice_viewer::viewer_set(self, h, located.figure, args);
        }
        if let Some(index) = located.textbox {
            return self.set_textbox_properties(located.figure, index, args);
        }
        if let Some(index) = located.annotation {
            return self.set_annotation_properties(located.figure, index, args);
        }
        let surface_rest = self.surface_set(h, located.figure, args)?;
        let args = match &surface_rest {
            Some(rest) if rest.is_empty() => return Ok(()),
            Some(rest) => rest.as_slice(),
            None => args,
        };
        if matches!(h.kind(), GraphicsKind::Series | GraphicsKind::Axes)
            && self.p2_set(h, located.figure, args)?
        {
            return Ok(());
        }
        let mut figure = self.graphics_figure(located.figure)?.clone();
        // Text, colorbar, colormap, yyaxis and data properties are applied to
        // the same candidate; the remaining pairs follow the original path.
        let ruler_sync = if h.kind() == GraphicsKind::NumericRuler {
            let has_limits = args.as_chunks::<2>().0.iter().any(|pair| {
                matches!(
                    text(&pair[0]).map(|s| s.to_ascii_lowercase()).as_deref(),
                    Ok("limits" | "limitsmode")
                )
            });
            if has_limits {
                h.axes.as_ref().and_then(|axes_id| {
                    figure
                        .all_axes()
                        .find(|(_, a)| a.identity.same_object(axes_id))
                        .and_then(|(_, a)| a.extras.ruler_index(&h.object))
                        .map(|axis| (axes_id.clone(), axis == 0, axis == 1))
                })
            } else {
                None
            }
        } else {
            None
        };
        let remaining = dot_syntax::set_extended(self, h, &mut figure, args)?;
        let args = remaining.as_slice();
        if args.is_empty() {
            if surface_commands::has_indexed_colors(&figure) {
                self.gobj_publish(located.figure, figure)?;
            } else {
                figure.validate()?;
                #[cfg(test)]
                tests::before_property_publish(&self.cancel);
                self.check_cancel()?;
                self.publish_graphics_figure(located.figure, figure);
            }
            if let Some((axes_id, sync_x, sync_y)) = ruler_sync {
                super::graphics::sync_engine_linked_axes(self, &axes_id, sync_x, sync_y);
            }
            return Ok(());
        }
        let mut sync_x = false;
        let mut sync_y = false;
        if h.kind() == GraphicsKind::Figure {
            let fig_key = figure.identity.recovery_key();
            for pair in args.as_chunks::<2>().0 {
                let raw = text(&pair[0])?;
                if let Some((super::graphics_root::DefaultQueryKind::Default, obj_type, canon)) =
                    super::graphics_root::parse_default_or_factory(raw)?
                {
                    if matches!(&pair[1], Value::Text(s) if s.eq_ignore_ascii_case("remove")) {
                        if let Some(m) = self.figure_defaults.get_mut(&fig_key) {
                            m.remove(&(obj_type, canon));
                        }
                    } else {
                        let value = super::graphics_root::DefaultProperty::parse(
                            obj_type,
                            canon,
                            &pair[1],
                            &self.cancel,
                        )?;
                        self.figure_defaults
                            .entry(fig_key)
                            .or_default()
                            .insert((obj_type, canon), value);
                    }
                    continue;
                }
                if raw.eq_ignore_ascii_case("Visible") {
                    let vis = match text(&pair[1])?.to_ascii_lowercase().as_str() {
                        "on" => true,
                        "off" => false,
                        _ => return Err("Visible supports on or off".into()),
                    };
                    figure.visible = vis;
                    self.set_figure_visible(&figure.identity, vis);
                    continue;
                }
                if !raw.eq_ignore_ascii_case("CurrentAxes") {
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
                    if kind == crate::plot::PlotKind::Scatter {
                        super::plot_properties::refresh_scatter_clim(&mut figure.axes);
                    }
                }
            } else {
                (sync_x, sync_y) = self.graphics_apply_axes_pairs(&mut figure, args)?;
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
        if surface_commands::has_indexed_colors(&figure)
            || figure.all_axes().any(|(_, axes)| {
                axes.series.iter().any(|s| {
                    s.kind == crate::plot::PlotKind::Patch
                        && s.face_color_mode == crate::plot::FaceColorMode::Flat
                        && (s.patch_cdata.is_some() || s.patch_vertex_cdata.is_some())
                })
            })
        {
            self.gobj_publish(located.figure, figure)?;
        } else {
            figure.prepare_legend_layout()?;
            figure.validate()?;
            #[cfg(test)]
            tests::before_property_publish(&self.cancel);
            self.check_cancel()?;
            self.publish_graphics_figure(located.figure, figure);
        }
        if (sync_x || sync_y)
            && h.kind() == GraphicsKind::Axes
            && let Some(axes_id) = h.axes.as_ref()
        {
            super::graphics::sync_engine_linked_axes(self, axes_id, sync_x, sync_y);
        }
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
