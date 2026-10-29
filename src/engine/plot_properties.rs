//! Bounded trailing graphics properties, validated before constructing sample buffers.
use super::Value;
use crate::{
    Result,
    plot::{LinePattern, PlotKind, Series},
};

#[derive(Default)]
pub(super) struct Properties {
    width: Option<f64>,
    group_width: Option<f64>,
    bar_width: Option<f64>,
    marker: Option<Option<crate::plot::MarkerShape>>,
    marker_size: Option<f64>,
    color: Option<[u8; 3]>,
    pattern: Option<LinePattern>,
    label: Option<String>,
    visible: Option<bool>,
    base_value: Option<f64>,
    marker_face: Option<Option<[u8; 3]>>,
    face_edge: Option<Option<[u8; 3]>>,
}
impl Properties {
    pub fn parse(args: &[Value], kind: PlotKind) -> Result<(usize, Self)> {
        let start = args
            .iter()
            .position(|value| match value {
                Value::Text(text) => text != "filled" && !is_line_spec(text),
                _ => false,
            })
            .unwrap_or(args.len());
        let tail = &args[start..];
        if !tail.len().is_multiple_of(2) {
            return Err("Plot properties require trailing name/value pairs".into());
        }
        if tail.len() > 32 {
            return Err("At most 16 plot property pairs are supported".into());
        }
        let mut result = Self::default();
        // scatter(X,Y,S,...) uses scalar area S, not a third dataset.
        // Validate before constructing any sample buffers; trailing SizeData
        // below may explicitly override this positional value.
        if kind == PlotKind::Scatter && start >= 3 && !matches!(&args[2], Value::Text(_)) {
            let Value::Matrix(area) = &args[2] else {
                return Err("scatter positional size requires one real scalar".into());
            };
            if start != 3 {
                return Err("scatter supports X,Y and optional scalar size; positional colors and additional datasets are unsupported".into());
            }
            let size = area.as_scalar()?.sqrt();
            crate::plot::validate_marker_size(size)?;
            result.marker_size = Some(size);
        }
        for pair in tail.as_chunks::<2>().0 {
            let name = text(&pair[0])?;
            if name.len() > 32 {
                return Err("Plot property name exceeds 32 bytes".into());
            }
            match name.to_ascii_lowercase().as_str() {
                "groupwidth" | "barwidth" if kind == PlotKind::Bar => {
                    let Value::Matrix(value) = &pair[1] else {
                        return Err("BarWidth and GroupWidth require real scalars".into());
                    };
                    let value = value.as_scalar()?;
                    if !value.is_finite() || !(0. < value && value <= 1.) {
                        return Err(
                            "BarWidth and GroupWidth currently require values in (0,1]".into()
                        );
                    }
                    if name.eq_ignore_ascii_case("groupwidth") {
                        result.group_width = Some(value);
                    } else {
                        result.bar_width = Some(value);
                    }
                }
                "basevalue" if kind == PlotKind::Stem => {
                    let Value::Matrix(value) = &pair[1] else {
                        return Err("BaseValue requires a real scalar".into());
                    };
                    let value = value.as_scalar()?;
                    if !value.is_finite() {
                        return Err("BaseValue must be finite".into());
                    }
                    result.base_value = Some(value);
                }
                "markerfacecolor" if kind != PlotKind::Bar => {
                    result.marker_face = Some(
                        if matches!(&pair[1],Value::Text(s) if s.eq_ignore_ascii_case("none")) {
                            None
                        } else {
                            Some(color(&pair[1])?)
                        },
                    );
                }
                "edgecolor" if kind == PlotKind::Bar => {
                    result.face_edge = Some(
                        if matches!(&pair[1],Value::Text(s) if s.eq_ignore_ascii_case("none")) {
                            None
                        } else {
                            Some(color(&pair[1])?)
                        },
                    );
                }
                "linewidth" => {
                    let Value::Matrix(value) = &pair[1] else {
                        return Err("LineWidth requires a numeric scalar".into());
                    };
                    let width = value.as_scalar()?;
                    crate::plot::validate_line_width(width)?;
                    result.width = Some(width);
                }
                "color" if matches!(kind, PlotKind::Line | PlotKind::Stairs | PlotKind::Stem) => {
                    result.color = Some(color(&pair[1])?);
                }
                "facecolor" if kind == PlotKind::Bar => result.color = Some(color(&pair[1])?),
                "markeredgecolor" if kind == PlotKind::Scatter => {
                    result.color = Some(color(&pair[1])?)
                }
                "linestyle"
                    if matches!(
                        kind,
                        PlotKind::Line | PlotKind::Stairs | PlotKind::Stem | PlotKind::Bar
                    ) =>
                {
                    result.pattern = Some(match text(&pair[1])? {
                        "-" => LinePattern::Solid,
                        "--" => LinePattern::Dashed,
                        ":" => LinePattern::Dotted,
                        "-." => LinePattern::DashDot,
                        "none" => LinePattern::None,
                        _ => return Err("LineStyle supports -, --, :, -. or none".into()),
                    });
                }
                "marker" if kind != PlotKind::Bar => {
                    let value = text(&pair[1])?;
                    result.marker = Some(if value == "none" {
                        None
                    } else {
                        Some(crate::plot::MarkerShape::parse(value)?)
                    });
                    if kind == PlotKind::Scatter && result.marker == Some(None) {
                        return Err("Scatter Marker none is not supported".into());
                    }
                }
                "markersize" if kind != PlotKind::Bar && kind != PlotKind::Scatter => {
                    let Value::Matrix(value) = &pair[1] else {
                        return Err("MarkerSize requires a numeric scalar".into());
                    };
                    let size = value.as_scalar()?;
                    crate::plot::validate_marker_size(size)?;
                    result.marker_size = Some(size);
                }
                "sizedata" if kind == PlotKind::Scatter => {
                    let Value::Matrix(value) = &pair[1] else {
                        return Err("SizeData requires a numeric scalar".into());
                    };
                    let size = value.as_scalar()?.sqrt();
                    crate::plot::validate_marker_size(size)?;
                    result.marker_size = Some(size);
                }
                "displayname" => {
                    let label = text(&pair[1])?;
                    if label.len() > 4096 {
                        return Err("Plot label exceeds 4096 bytes".into());
                    }
                    result.label = Some(label.to_owned());
                }
                "visible" => {
                    result.visible = Some(match &pair[1] {
                        Value::Text(value) if value.eq_ignore_ascii_case("on") => true,
                        Value::Text(value) if value.eq_ignore_ascii_case("off") => false,
                        Value::Logical(value) => value.as_scalar()?,
                        Value::Matrix(value) if value.as_scalar()? == 1. => true,
                        Value::Matrix(value) if value.as_scalar()? == 0. => false,
                        _ => return Err("Visible requires on, off, 1 or 0".into()),
                    });
                }
                _ => return Err(format!("Unsupported plot property '{name}' for {kind:?}")),
            }
        }
        if result.bar_width.is_some() && result.group_width.is_none() {
            return Err("BarWidth currently requires explicit GroupWidth; automatic group width is unsupported".into());
        }
        Ok((start, result))
    }
    /// Construction applies geometry to each actual batch, before figure publication.
    pub fn apply_new_batch(
        &self,
        series: &mut [Series],
        kind: PlotKind,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<()> {
        crate::dsp::check_cancel(cancel)?;
        if self.group_width.is_some() || self.bar_width.is_some() {
            let count = series.len();
            if kind != PlotKind::Bar || count > crate::plot::MAX_SERIES {
                return Err("Relative Bar geometry requires a bounded Bar batch".into());
            }
            for (column, s) in series.iter_mut().enumerate() {
                let mut previous = None;
                let mut spacing = None::<f64>;
                for (index, point) in s.points.iter().enumerate() {
                    if index.is_multiple_of(1024) {
                        crate::dsp::check_cancel(cancel)?;
                    }
                    if let Some(x) = previous {
                        let delta = point[0] - x;
                        spacing = Some(spacing.map_or(delta, |old| old.min(delta)));
                    }
                    previous = Some(point[0]);
                }
                let pitch = spacing.unwrap_or(1.) * self.group_width.unwrap_or(0.8) / count as f64;
                // MATLAB documents an omitted BarWidth of 0.8. The exporter
                // supplies both properties explicitly; no-property legacy geometry
                // never enters this branch. Automatic GroupWidth is not inferred.
                let width = pitch * self.bar_width.unwrap_or(0.8);
                let offset = (column as f64 - (count as f64 - 1.) / 2.) * pitch;
                if !width.is_finite() || width <= 0. || !offset.is_finite() {
                    return Err("Relative Bar geometry cannot be represented".into());
                }
                s.bar_width = width;
                s.bar_offset = offset;
            }
        }
        self.apply_style(series, kind)?;
        crate::dsp::check_cancel(cancel)
    }
    pub fn apply(&self, series: &mut [Series], kind: PlotKind) -> Result<()> {
        if self.group_width.is_some() || self.bar_width.is_some() {
            return Err("BarWidth and GroupWidth currently require Bar construction; editing an existing group is unsupported".into());
        }
        self.apply_style(series, kind)
    }
    fn apply_style(&self, series: &mut [Series], kind: PlotKind) -> Result<()> {
        for s in series {
            if let Some(edge) = self.face_edge {
                s.face_edge_color = edge;
            }
            if let Some(width) = self.width {
                s.line_width = Some(width);
            }
            if let Some(color) = self.color {
                s.color = color;
                s.explicit_color = true;
            }
            if let Some(pattern) = self.pattern {
                s.pattern = pattern;
                // A marker-only LineSpec can be overridden by an explicit line style.
                if kind == PlotKind::Line {
                    s.kind = PlotKind::Line;
                }
            }
            if let Some(marker) = self.marker {
                if marker.is_none() && s.kind == PlotKind::Scatter {
                    return Err("Marker none requires a visible line style".into());
                }
                s.markers = marker.is_some();
                s.marker_shape = marker;
                s.marker_filled = marker.is_some()
                    && (s.marker_filled || marker == Some(crate::plot::MarkerShape::Point));
            }
            if let Some(size) = self.marker_size {
                s.marker_size = Some(size);
            }
            if let Some(label) = &self.label {
                s.label.clone_from(label);
            }
            if let Some(visible) = self.visible {
                s.visible = visible;
            }
            if let Some(base) = self.base_value {
                s.base_value = Some(base);
                s.validate_base_value()?;
            }
            if let Some(fill) = self.marker_face {
                if fill.is_some_and(|color| !s.explicit_color || color != s.color) {
                    return Err("MarkerFaceColor currently supports none or an explicitly selected matching series color; independent/automatic fill colors are unsupported".into());
                }
                s.marker_filled = fill.is_some();
            }
        }
        Ok(())
    }
}
fn text(value: &Value) -> Result<&str> {
    if let Value::Text(text) = value {
        Ok(text)
    } else {
        Err("Plot property requires quoted text".into())
    }
}
fn is_line_spec(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 16
        && text.bytes().all(|c| b"rgbcmykwo.-:+x*sd^v<>".contains(&c))
}
pub(super) fn color(value: &Value) -> Result<[u8; 3]> {
    if let Value::Matrix(rgb) = value {
        if rgb.len() == 3
            && rgb
                .data()
                .iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(v))
        {
            return Ok(std::array::from_fn(|i| {
                (rgb.data()[i] * 255.).round() as u8
            }));
        }
        return Err("Color requires three finite RGB values in 0..1".into());
    }
    let name = text(value)?;
    if name.len() > 9 {
        return Err("Unsupported color name".into());
    }
    let name = name.to_ascii_lowercase();
    let rgb = match name.as_str() {
        "r" | "red" => [255, 0, 0],
        "g" | "green" => [0, 255, 0],
        "b" | "blue" => [0, 0, 255],
        "c" | "cyan" => [0, 255, 255],
        "m" | "magenta" => [255, 0, 255],
        "y" | "yellow" => [255, 255, 0],
        "k" | "black" => [0, 0, 0],
        "w" | "white" => [255, 255, 255],
        _ => {
            let hex = name.strip_prefix('#').ok_or("Unsupported color name")?;
            if !matches!(hex.len(), 3 | 6) || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err("Hex colors require #RGB or #RRGGBB".into());
            }
            let mut rgb = [0; 3];
            for (i, channel) in rgb.iter_mut().enumerate() {
                *channel = if hex.len() == 3 {
                    u8::from_str_radix(&hex[i..i + 1], 16).unwrap() * 17
                } else {
                    u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap()
                };
            }
            rgb
        }
    };
    Ok(rgb)
}
