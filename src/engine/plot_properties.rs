//! Bounded trailing graphics properties, validated before constructing sample buffers.
use super::Value;
use crate::{
    Result,
    plot::{
        BarLayout, FaceColorMode, LinePattern, MarkerColor, PlotKind, ScatterCData, Series,
        validate_alpha, validate_marker_area,
    },
};
use std::sync::Arc;

#[derive(Clone)]
enum SizeData {
    Scalar(f64),
    PerPoint(Arc<[f64]>),
}
#[derive(Clone)]
enum CData {
    Single([u8; 3]),
    PerPoint(ScatterCData),
}
#[derive(Default)]
pub(super) struct Properties {
    width: Option<f64>,
    group_width: Option<f64>,
    bar_width: Option<f64>,
    bar_layout: Option<BarLayout>,
    marker: Option<Option<crate::plot::MarkerShape>>,
    marker_size: Option<f64>,
    size_data: Option<SizeData>,
    color: Option<[u8; 3]>,
    cdata: Option<CData>,
    face_color_mode: Option<FaceColorMode>,
    face_alpha: Option<f64>,
    bar_cdata: Option<Vec<[u8; 3]>>,
    pattern: Option<LinePattern>,
    label: Option<String>,
    visible: Option<bool>,
    base_value: Option<f64>,
    marker_face: Option<MarkerColor>,
    marker_edge: Option<MarkerColor>,
    marker_face_alpha: Option<f64>,
    marker_edge_alpha: Option<f64>,
    face_edge: Option<Option<[u8; 3]>>,
    /// scatter / scatter3 'filled'.
    filled: Option<bool>,
}
/// scatter and scatter3 reject a positional line style such as 'r-o'.
pub(super) const LINE_SPEC_UNSUPPORTED: &str =
    "scatter/bar support a single color letter; full line specs require plot";
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
                "markerfacecolor" if kind == PlotKind::Scatter => {
                    result.marker_face = Some(parse_marker_color(&pair[1], true)?);
                }
                "markerfacecolor" if kind != PlotKind::Bar => {
                    result.marker_face = Some(
                        if matches!(&pair[1], Value::Text(s) if s.eq_ignore_ascii_case("none")) {
                            MarkerColor::None
                        } else {
                            MarkerColor::Rgb(color(&pair[1])?)
                        },
                    );
                }
                "markeredgecolor" if kind == PlotKind::Scatter => {
                    result.marker_edge = Some(parse_marker_color(&pair[1], false)?);
                }
                "markerfacealpha" if kind == PlotKind::Scatter => {
                    let Value::Matrix(value) = &pair[1] else {
                        return Err("MarkerFaceAlpha requires a numeric scalar".into());
                    };
                    let alpha = value.as_scalar()?;
                    validate_alpha(alpha, "MarkerFaceAlpha")?;
                    result.marker_face_alpha = Some(alpha);
                }
                "markeredgealpha" if kind == PlotKind::Scatter => {
                    let Value::Matrix(value) = &pair[1] else {
                        return Err("MarkerEdgeAlpha requires a numeric scalar".into());
                    };
                    let alpha = value.as_scalar()?;
                    validate_alpha(alpha, "MarkerEdgeAlpha")?;
                    result.marker_edge_alpha = Some(alpha);
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
                "barlayout" if kind == PlotKind::Bar => {
                    result.bar_layout = Some(parse_bar_layout(&pair[1])?);
                }
                "facecolor" if kind == PlotKind::Bar => match &pair[1] {
                    Value::Text(s) if s.eq_ignore_ascii_case("flat") => {
                        result.face_color_mode = Some(FaceColorMode::Flat);
                    }
                    Value::Text(s) if s.eq_ignore_ascii_case("none") => {
                        result.face_color_mode = Some(FaceColorMode::None);
                    }
                    _ => {
                        result.color = Some(color(&pair[1])?);
                        result.face_color_mode = Some(FaceColorMode::Color);
                    }
                },
                "facealpha" if kind == PlotKind::Bar => {
                    let Value::Matrix(value) = &pair[1] else {
                        return Err("FaceAlpha requires a real scalar in 0..1".into());
                    };
                    let alpha = value.as_scalar()?;
                    if !alpha.is_finite() || !(0. ..=1.).contains(&alpha) {
                        return Err("FaceAlpha must be a scalar in 0..1".into());
                    }
                    result.face_alpha = Some(alpha);
                }
                "cdata" if kind == PlotKind::Bar => {
                    let Value::Matrix(m) = &pair[1] else {
                        return Err(
                            "CData requires a real n-by-3 matrix with values in [0,1]".into()
                        );
                    };
                    if m.cols() != 3 || m.is_empty() {
                        return Err("CData requires a non-empty n-by-3 matrix".into());
                    }
                    let mut rows = Vec::with_capacity(m.rows());
                    for r in 0..m.rows() {
                        let rgb = [m.get(r, 0), m.get(r, 1), m.get(r, 2)];
                        if rgb
                            .iter()
                            .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
                        {
                            return Err("CData values must be in [0,1]".into());
                        }
                        rows.push(rgb.map(|v| (v * 255.).round() as u8));
                    }
                    result.bar_cdata = Some(rows);
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
                    result.size_data = Some(parse_size_data(&pair[1])?);
                }
                "cdata" if kind == PlotKind::Scatter => {
                    result.cdata = Some(parse_cdata(&pair[1])?);
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
        Ok((start, result))
    }
    /// bar(Y[,width][,style][,Name,Value...]) or bar(x,Y[,width][,style][,Name,Value...]).
    pub fn parse_bar(args: &[Value]) -> Result<(usize, Self)> {
        if args.is_empty() {
            return Err("Plot requires data".into());
        }
        let num_data = args
            .iter()
            .take_while(|v| super::plot_time_axes::is_plot_data_arg(v))
            .count();
        if num_data == 0 {
            return Err("Plot requires numeric coordinate arrays".into());
        }
        let mut width = None;
        let mut layout = None;
        let mut bar_color = None;
        let (coord_end, mut idx) = if num_data == 2
            && matches!(&args[1], Value::Matrix(second) if second.len() == 1)
            && matches!(&args[0], Value::Matrix(first) if first.len() > 1)
        {
            (1, 1)
        } else if num_data >= 3
            && num_data % 2 == 1
            && matches!(&args[num_data - 1], Value::Matrix(last) if last.len() == 1)
        {
            (num_data - 1, num_data - 1)
        } else {
            (num_data, num_data)
        };
        while idx < args.len() {
            match &args[idx] {
                Value::Matrix(w_mat) if width.is_none() && w_mat.len() == 1 => {
                    let w = w_mat.as_scalar()?;
                    if !w.is_finite() || !(0. < w && w <= 1.) {
                        return Err("BarWidth requires a value in (0,1]".into());
                    }
                    width = Some(w);
                    idx += 1;
                }
                Value::Text(s) if layout.is_none() && (s.eq_ignore_ascii_case("grouped") || s.eq_ignore_ascii_case("stacked")) => {
                    layout = Some(parse_bar_layout(&args[idx])?);
                    idx += 1;
                }
                Value::Text(s) if bar_color.is_none() && s.len() == 1 && "rgbcmykw".contains(s.as_str()) => {
                    bar_color = Some(color(&args[idx])?);
                    idx += 1;
                }
                Value::Text(s) if is_line_spec(s) => {
                    return Err(LINE_SPEC_UNSUPPORTED.into());
                }
                Value::Text(_) => break,
                _ => return Err("bar accepts Y or (x,Y), optional width, 'grouped'/'stacked', color letter, and Name,Value pairs".into()),
            }
        }
        let (_, mut result) = Self::parse(&args[idx..], PlotKind::Bar)?;
        if result.bar_width.is_none() {
            result.bar_width = width;
        }
        if result.bar_layout.is_none() {
            result.bar_layout = layout;
        }
        if result.color.is_none() && result.face_color_mode.is_none() {
            result.color = bar_color;
        }
        Ok((coord_end, result))
    }
    /// Shared parser for `scatter(X,Y[,S[,C]][,'filled'][,marker][,Name,Value...])`
    /// (`coord_count = 2`) and `scatter3(X,Y,Z,...)` (`coord_count = 3`).
    pub fn parse_scatter(args: &[Value], coord_count: usize) -> Result<(usize, Self)> {
        let mut start = coord_count.min(args.len());
        let mut pos_size = None;
        let mut pos_color = None;
        let mut marker = None;
        let mut filled = None;
        let mut saw_size = false;
        let mut saw_color = false;
        while start < args.len() {
            match &args[start] {
                Value::Matrix(m) if !saw_size && start == coord_count => {
                    saw_size = true;
                    if !m.is_empty() {
                        pos_size = Some(parse_size_data(&args[start])?);
                    }
                }
                Value::Logical(_) if !saw_size && start == coord_count => {
                    saw_size = true;
                    pos_size = Some(parse_size_data(&args[start])?);
                }
                Value::Matrix(m) if saw_size && !saw_color && start == coord_count + 1 => {
                    saw_color = true;
                    if !m.is_empty() {
                        pos_color = Some(parse_cdata(&args[start])?);
                    }
                }
                Value::Text(text) if text.eq_ignore_ascii_case("filled") => filled = Some(true),
                Value::Text(text) if crate::plot::MarkerShape::parse(text).is_ok() => {
                    marker = Some(crate::plot::MarkerShape::parse(text)?);
                }
                Value::Text(_) if pos_color.is_none() && self::color(&args[start]).is_ok() => {
                    pos_color = Some(CData::Single(self::color(&args[start])?));
                }
                // A line style such as 'r-o' is rejected, never dropped silently.
                Value::Text(text) if is_line_spec(text) => {
                    return Err(LINE_SPEC_UNSUPPORTED.into());
                }
                Value::Text(_) => break,
                _ => {
                    return Err(if coord_count == 3 {
                        "scatter3 accepts X, Y, Z, optional S and C, 'filled', a marker and Name,Value pairs"
                    } else {
                        "scatter accepts X, Y, optional S and C, 'filled', a marker and Name,Value pairs"
                    }
                    .into());
                }
            }
            start += 1;
        }
        let (_, mut result) = Self::parse(&args[start..], PlotKind::Scatter)?;
        if result.size_data.is_none() {
            result.size_data = pos_size;
        }
        if result.cdata.is_none() {
            result.cdata = pos_color;
        }
        if marker.is_some() && result.marker.is_none() {
            result.marker = Some(marker);
        }
        if result.filled.is_none() {
            result.filled = filled;
        }
        Ok((coord_count.min(args.len()), result))
    }
    /// Construction applies geometry to each actual batch, before figure publication.
    pub fn apply_new_batch(
        &self,
        series: &mut [Series],
        kind: PlotKind,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<()> {
        crate::dsp::check_cancel(cancel)?;
        if self.group_width.is_some()
            || self.bar_width.is_some()
            || self.bar_layout == Some(BarLayout::Stacked)
        {
            let count = series.len();
            if kind != PlotKind::Bar || count > crate::plot::MAX_SERIES {
                return Err("Relative Bar geometry requires a bounded Bar batch".into());
            }
            let layout = self.bar_layout.unwrap_or(BarLayout::Grouped);
            for (column, s) in series.iter_mut().enumerate() {
                s.bar_layout = layout;
                s.bar_relative_width = self.bar_width;
                s.bar_group_width = self.group_width;
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
                let spacing = spacing.unwrap_or(1.);
                let (width, offset) = match layout {
                    BarLayout::Stacked => (spacing * self.bar_width.unwrap_or(0.8), 0.),
                    BarLayout::Grouped => {
                        if count == 1 && self.group_width.is_none() {
                            (spacing * self.bar_width.unwrap_or(0.8), 0.)
                        } else {
                            let pitch = spacing * self.group_width.unwrap_or(0.8) / count as f64;
                            let width = pitch * self.bar_width.unwrap_or(0.8);
                            let offset = (column as f64 - (count as f64 - 1.) / 2.) * pitch;
                            (width, offset)
                        }
                    }
                };
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
                if kind == PlotKind::Bar && self.face_color_mode.is_none() {
                    s.face_color_mode = FaceColorMode::Color;
                }
            }
            if let Some(mode) = self.face_color_mode {
                s.face_color_mode = mode;
            }
            if let Some(alpha) = self.face_alpha {
                s.face_alpha = Some(alpha);
            }
            if let Some(cdata) = &self.bar_cdata {
                if cdata.len() != s.points.len() {
                    return Err(format!(
                        "Bar CData must have {} rows to match YData",
                        s.points.len()
                    ));
                }
                s.face_colors = Some(std::sync::Arc::from(cdata.as_slice()));
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
                s.marker_sizes = None;
            }
            if let Some(size_data) = &self.size_data {
                match size_data {
                    SizeData::Scalar(pts) => {
                        s.marker_size = Some(*pts);
                        s.marker_sizes = None;
                    }
                    SizeData::PerPoint(areas) => {
                        if areas.len() != s.points.len() {
                            return Err("SizeData must match XData/YData length".into());
                        }
                        s.marker_sizes = Some(areas.clone());
                        s.marker_size = None;
                    }
                }
            }
            if let Some(cdata) = &self.cdata {
                match cdata {
                    CData::Single(rgb) => {
                        s.color = *rgb;
                        s.explicit_color = true;
                        s.scatter_cdata = None;
                    }
                    CData::PerPoint(cd) => {
                        let len = match cd {
                            ScatterCData::Rgb(rgbs) => {
                                if let Some(first) = rgbs.first() {
                                    s.color = *first;
                                }
                                rgbs.len()
                            }
                            ScatterCData::Indexed(vals) => vals.len(),
                        };
                        if len != s.points.len() {
                            return Err("CData must match XData/YData length".into());
                        }
                        s.scatter_cdata = Some(cd.clone());
                        s.explicit_color = true;
                    }
                }
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
            if let Some(filled) = self.filled {
                if filled && !s.marker_filled && s.marker_shape.is_none() {
                    s.marker_shape = Some(crate::plot::MarkerShape::Circle);
                }
                s.marker_filled = filled;
                if filled {
                    if s.marker_face_color == MarkerColor::Default {
                        s.marker_face_color = MarkerColor::Flat;
                    }
                    if s.kind == PlotKind::Scatter && s.marker_edge_color == MarkerColor::Default {
                        s.marker_edge_color = MarkerColor::None;
                    }
                }
            }
            if let Some(fill) = self.marker_face {
                if s.kind == PlotKind::Scatter {
                    s.marker_face_color = fill;
                    s.marker_filled = !matches!(fill, MarkerColor::None);
                } else {
                    if let MarkerColor::Rgb(color) = fill
                        && (!s.explicit_color || color != s.color)
                    {
                        return Err("MarkerFaceColor currently supports none or an explicitly selected matching series color; independent/automatic fill colors are unsupported".into());
                    }
                    s.marker_filled = !matches!(fill, MarkerColor::None);
                }
            }
            if let Some(edge) = self.marker_edge {
                s.marker_edge_color = edge;
                if s.kind != PlotKind::Scatter
                    && let MarkerColor::Rgb(rgb) = edge
                    && s.scatter_cdata.is_none()
                    && self.cdata.is_none()
                {
                    s.color = rgb;
                    s.explicit_color = true;
                }
            }
            if let Some(alpha) = self.marker_face_alpha {
                s.marker_face_alpha = Some(alpha);
            }
            if let Some(alpha) = self.marker_edge_alpha {
                s.marker_edge_alpha = Some(alpha);
            }
        }
        Ok(())
    }
}
pub(super) fn parse_bar_layout(value: &Value) -> Result<BarLayout> {
    let s = text(value)?;
    if s.eq_ignore_ascii_case("grouped") {
        Ok(BarLayout::Grouped)
    } else if s.eq_ignore_ascii_case("stacked") {
        Ok(BarLayout::Stacked)
    } else {
        Err("BarLayout must be 'grouped' or 'stacked'".into())
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
pub(super) fn parse_marker_color(value: &Value, allow_auto: bool) -> Result<MarkerColor> {
    if let Value::Text(s) = value {
        if s.eq_ignore_ascii_case("none") {
            return Ok(MarkerColor::None);
        }
        if s.eq_ignore_ascii_case("flat") {
            return Ok(MarkerColor::Flat);
        }
        if s.eq_ignore_ascii_case("auto") {
            return if allow_auto {
                Ok(MarkerColor::Auto)
            } else {
                Err("MarkerEdgeColor supports 'none', 'flat', or an RGB color".into())
            };
        }
    }
    Ok(MarkerColor::Rgb(color(value)?))
}
fn parse_size_data(value: &Value) -> Result<SizeData> {
    match value {
        Value::Matrix(m) if (m.rows() == 1 || m.cols() == 1) && !m.is_empty() => {
            if m.len() == 1 {
                let area = m.data()[0];
                validate_marker_area(area)?;
                Ok(SizeData::Scalar(area.sqrt()))
            } else {
                for &area in m.data() {
                    validate_marker_area(area)?;
                }
                Ok(SizeData::PerPoint(Arc::from(m.data())))
            }
        }
        Value::Logical(l) => {
            let area = if l.as_scalar()? { 1. } else { 0. };
            validate_marker_area(area)?;
            Ok(SizeData::Scalar(area.sqrt()))
        }
        _ => Err("SizeData requires a positive finite numeric scalar or vector".into()),
    }
}
fn parse_cdata(value: &Value) -> Result<CData> {
    if let Value::Text(_) = value {
        return Ok(CData::Single(color(value)?));
    }
    let Value::Matrix(m) = value else {
        return Err(
            "CData requires a color, RGB triplet, n-by-3 RGB matrix, or n-element vector".into(),
        );
    };
    if m.is_empty() {
        return Err("CData cannot be empty".into());
    }
    if m.rows() == 1 && m.cols() == 3 {
        return Ok(CData::Single(color(value)?));
    }
    if m.cols() == 3 && m.rows() > 1 {
        let n = m.rows();
        let data = m.data();
        if !data.iter().all(|v| v.is_finite() && (0. ..=1.).contains(v)) {
            return Err("CData RGB matrix requires finite values in 0..1".into());
        }
        let rgbs: Vec<[u8; 3]> = (0..n)
            .map(|r| {
                [
                    (data[r] * 255.).round() as u8,
                    (data[r + n] * 255.).round() as u8,
                    (data[r + 2 * n] * 255.).round() as u8,
                ]
            })
            .collect();
        return Ok(CData::PerPoint(ScatterCData::Rgb(rgbs.into())));
    }
    if m.rows() == 1 || m.cols() == 1 {
        if m.data().iter().any(|v| v.is_infinite()) {
            return Err("CData vector values must be finite".into());
        }
        return Ok(CData::PerPoint(ScatterCData::Indexed(Arc::from(m.data()))));
    }
    Err("CData requires an RGB triplet, n-by-3 RGB matrix, or n-element vector".into())
}
/// Recompute the automatic CLim from any Scatter series with indexed CData,
/// keeping `axes.extras.clim` unset so `CLimMode` stays `'auto'`.
pub(crate) fn refresh_scatter_clim(axes: &mut crate::plot::Axes) {
    let mut range: Option<[f64; 2]> = None;
    for s in &axes.series {
        if let Some(ScatterCData::Indexed(vals)) = &s.scatter_cdata {
            for &v in vals.iter() {
                if v.is_finite() {
                    range = Some(range.map_or([v, v], |r| [r[0].min(v), r[1].max(v)]));
                }
            }
        }
        if s.kind == PlotKind::Patch
            && s.face_color_mode == FaceColorMode::Flat
            && let Some(v) = s.patch_cdata
            && v.is_finite()
        {
            range = Some(range.map_or([v, v], |r| [r[0].min(v), r[1].max(v)]));
        }
        if s.kind == PlotKind::Patch
            && s.face_color_mode == FaceColorMode::Flat
            && let Some(vertex) = &s.patch_vertex_cdata
        {
            for &v in vertex.source.iter().filter(|v| v.is_finite()) {
                range = Some(range.map_or([v, v], |r| [r[0].min(v), r[1].max(v)]));
            }
        }
    }
    axes.extras.auto_clim = range.map(|[a, b]| if a < b { [a, b] } else { [a - 1., a + 1.] });
}

/// Recompute `Series.color` for scalar-CData patches in `'flat'` FaceColor mode.
pub(crate) fn refresh_scalar_patch_colors(axes: &mut crate::plot::Axes) {
    let map = axes.extras.effective_colormap();
    let clim = axes.color_limits();
    for s in &mut axes.series {
        if s.kind == PlotKind::Patch
            && s.face_color_mode == FaceColorMode::Flat
            && let Some(c) = s.patch_cdata
        {
            s.color = crate::plot::colormap::scaled_rgb(&map, clim, c);
        }
    }
}
