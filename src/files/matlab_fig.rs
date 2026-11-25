//! Bounded MATLAB FIG exchange for supported 2-D axes and series.
use super::fig_mat::{self as mat, Value};
use crate::{
    Result,
    plot::{AxisScale, COLORS, Figure, LinePattern, PlotKind, Series},
};
use std::{collections::BTreeMap, path::Path};
#[cfg(test)]
#[path = "fig_static_legend_writer_tests.rs"]
mod static_legend_writer_tests;
type Fields<'a> = BTreeMap<String, Value<'a>>;
#[cfg(test)]
#[path = "fig_axes_color_tests.rs"]
mod axes_color_tests;
#[cfg(test)]
#[path = "fig_bar_outline_tests.rs"]
mod bar_outline_tests;
#[cfg(test)]
#[path = "fig_dark_style_tests.rs"]
mod dark_style_tests;
#[cfg(test)]
#[path = "fig_marker_color_tests.rs"]
mod marker_color_tests;
#[cfg(test)]
#[path = "fig_legend_membership_tests.rs"]
mod membership_tests;
#[cfg(test)]
#[path = "fig_ordered_legend_tests.rs"]
mod ordered_legend_tests;
fn field<'a>(fields: &'a Fields<'a>, key: &str) -> Result<&'a Value<'a>> {
    fields
        .get(key)
        .ok_or_else(|| format!("FIG is missing {key}"))
}
fn object<'a>(value: &'a Value<'a>) -> Result<&'a Fields<'a>> {
    let s = value.structs().ok_or("Expected FIG structure")?;
    if s.len() != 1 {
        return Err("Expected a scalar FIG structure".into());
    }
    Ok(&s[0])
}
fn string<'a>(fields: &'a Fields<'a>, key: &str, default: &'a str) -> Result<&'a str> {
    fields.get(key).map_or(Ok(default), |v| {
        v.text().ok_or_else(|| format!("FIG {key} must be text"))
    })
}
fn scalar(fields: &Fields<'_>, key: &str, default: f64) -> Result<f64> {
    fields.get(key).map_or(Ok(default), |v| {
        if v.len() == 1 {
            v.number(0).ok_or_else(|| format!("Invalid FIG {key}"))
        } else {
            Err(format!("FIG {key} must be scalar"))
        }
    })
}
// MatRust-only kind information for its legacy line-encoded scatter export.
// Public LineStyle/Marker properties still carry the MATLAB-visible appearance.
fn scatter_hint(props: &Fields<'_>) -> Result<Option<LinePattern>> {
    let Some(items) = props.get("UserData").and_then(Value::structs) else {
        return Ok(None);
    };
    if !items.iter().any(|f| f.contains_key("MatRustGraphics")) {
        return Ok(None);
    }
    if items.len() != 1 {
        return Err("MatRustGraphics UserData must be scalar".into());
    }
    let hint = object(field(&items[0], "MatRustGraphics")?)
        .map_err(|_| "MatRustGraphics metadata must be a scalar structure".to_owned())?;
    if hint.len() != 3
        || scalar(hint, "Version", f64::NAN)? != 1.
        || string(hint, "Kind", "")? != "scatter"
    {
        return Err("Unsupported MatRustGraphics version, kind or fields".into());
    }
    Ok(Some(match string(hint, "Pattern", "")? {
        "Solid" => LinePattern::Solid,
        "Dashed" => LinePattern::Dashed,
        "Dotted" => LinePattern::Dotted,
        "DashDot" => LinePattern::DashDot,
        "None" => LinePattern::None,
        _ => return Err("Invalid MatRustGraphics Pattern".into()),
    }))
}
// MatRust chart helpers (error-bar segments, polar grid) are never legend
// members: MATLAB's Annotation.LegendInformation.IconDisplayStyle 'off'.
// FIG Annotation objects are not modelled, so the flag rides in UserData.
fn legend_hidden_hint(props: &Fields<'_>) -> Result<bool> {
    let Some(items) = props.get("UserData").and_then(Value::structs) else {
        return Ok(false);
    };
    if !items.iter().any(|f| f.contains_key("MatRustLegend")) {
        return Ok(false);
    }
    if items.len() != 1 {
        return Err("MatRustLegend UserData must be scalar".into());
    }
    let hint = object(field(&items[0], "MatRustLegend")?)
        .map_err(|_| "MatRustLegend metadata must be a scalar structure".to_owned())?;
    if hint.len() != 2 || scalar(hint, "Version", f64::NAN)? != 1. {
        return Err("Unsupported MatRustLegend version or fields".into());
    }
    match string(hint, "IconDisplayStyle", "")? {
        "off" => Ok(true),
        "on" => Ok(false),
        _ => Err("Invalid MatRustLegend IconDisplayStyle".into()),
    }
}
fn encode_marker_color(c: crate::plot::MarkerColor) -> mat::Encoded<'static> {
    match c {
        crate::plot::MarkerColor::Default => mat::text("default"),
        crate::plot::MarkerColor::None => mat::text("none"),
        crate::plot::MarkerColor::Flat => mat::text("flat"),
        crate::plot::MarkerColor::Auto => mat::text("auto"),
        crate::plot::MarkerColor::Rgb(rgb) => {
            mat::numbers(rgb.into_iter().map(|v| f64::from(v) / 255.))
        }
    }
}
fn decode_marker_color(value: &Value<'_>) -> Result<crate::plot::MarkerColor> {
    if let Some(t) = value.text() {
        return match t {
            "default" => Ok(crate::plot::MarkerColor::Default),
            "none" => Ok(crate::plot::MarkerColor::None),
            "flat" => Ok(crate::plot::MarkerColor::Flat),
            "auto" => Ok(crate::plot::MarkerColor::Auto),
            _ => Err("Invalid MatRustScatter marker color".into()),
        };
    }
    Ok(crate::plot::MarkerColor::Rgb(color(value)?))
}
fn apply_scatter_style_hint(props: &Fields<'_>, series: &mut Series) -> Result<()> {
    let Some(items) = props.get("UserData").and_then(Value::structs) else {
        return Ok(());
    };
    if !items.iter().any(|f| f.contains_key("MatRustScatter")) {
        return Ok(());
    }
    if items.len() != 1 {
        return Err("MatRustScatter UserData must be scalar".into());
    }
    let hint = object(field(&items[0], "MatRustScatter")?)
        .map_err(|_| "MatRustScatter metadata must be a scalar structure".to_owned())?;
    if scalar(hint, "Version", f64::NAN)? != 1. {
        return Err("Unsupported MatRustScatter version".into());
    }
    if let Some(sizes) = hint.get("SizeData") {
        let vals: Vec<f64> = (0..sizes.len()).filter_map(|i| sizes.number(i)).collect();
        if vals.len() != sizes.len() {
            return Err("Invalid MatRustScatter SizeData".into());
        }
        series.marker_sizes = Some(std::sync::Arc::from(vals));
    }
    if let Some(rgbs) = hint.get("CDataRgb") {
        if !rgbs.len().is_multiple_of(3) {
            return Err("Invalid MatRustScatter CDataRgb".into());
        }
        let mut list = Vec::with_capacity(rgbs.len() / 3);
        for i in 0..rgbs.len() / 3 {
            let mut rgb = [0u8; 3];
            for (c, out) in rgb.iter_mut().enumerate() {
                let v = rgbs
                    .number(3 * i + c)
                    .ok_or("Invalid MatRustScatter CDataRgb")?;
                if !v.is_finite() || !(0. ..=1.).contains(&v) {
                    return Err("Invalid MatRustScatter CDataRgb".into());
                }
                *out = (v * 255.).round() as u8;
            }
            list.push(rgb);
        }
        series.scatter_cdata = Some(crate::plot::ScatterCData::Rgb(std::sync::Arc::from(list)));
    } else if let Some(indexed) = hint.get("CDataIndexed") {
        let vals: Vec<f64> = (0..indexed.len())
            .filter_map(|i| indexed.number(i))
            .collect();
        if vals.len() != indexed.len() {
            return Err("Invalid MatRustScatter CDataIndexed".into());
        }
        series.scatter_cdata = Some(crate::plot::ScatterCData::Indexed(std::sync::Arc::from(
            vals,
        )));
    }
    if let Some(face) = hint.get("MarkerFaceColor") {
        series.marker_face_color = decode_marker_color(face)?;
        match series.marker_face_color {
            crate::plot::MarkerColor::None => series.marker_filled = false,
            crate::plot::MarkerColor::Default => {}
            _ => series.marker_filled = true,
        }
    }
    if let Some(edge) = hint.get("MarkerEdgeColor") {
        series.marker_edge_color = decode_marker_color(edge)?;
    }
    if hint.contains_key("MarkerFaceAlpha") {
        let a = scalar(hint, "MarkerFaceAlpha", f64::NAN)?;
        crate::plot::validate_alpha(a, "MarkerFaceAlpha")?;
        series.marker_face_alpha = Some(a);
    }
    if hint.contains_key("MarkerEdgeAlpha") {
        let a = scalar(hint, "MarkerEdgeAlpha", f64::NAN)?;
        crate::plot::validate_alpha(a, "MarkerEdgeAlpha")?;
        series.marker_edge_alpha = Some(a);
    }
    series.validate_scatter()?;
    Ok(())
}
// A single public FIG width cannot express the separate legacy line/marker
// defaults. Restore implicit semantics only while the public width is unchanged.
fn implicit_width_hint(props: &Fields<'_>, kind: PlotKind) -> Result<bool> {
    let Some(items) = props.get("UserData").and_then(Value::structs) else {
        return Ok(false);
    };
    if !items.iter().any(|f| f.contains_key("MatRustImplicitWidth")) {
        return Ok(false);
    }
    if items.len() != 1 {
        return Err("MatRustImplicitWidth UserData must be scalar".into());
    }
    let hint = object(field(&items[0], "MatRustImplicitWidth")?)
        .map_err(|_| "MatRustImplicitWidth metadata must be a scalar structure".to_owned())?;
    let exported = scalar(hint, "ExportedValue", f64::NAN)?;
    if hint.len() != 2
        || scalar(hint, "Version", f64::NAN)? != 1.
        || !matches!(exported, 0.75 | 1.5)
    {
        return Err("Unsupported MatRustImplicitWidth version, value or fields".into());
    }
    if kind != PlotKind::Line {
        return Err("MatRustImplicitWidth requires a line object".into());
    }
    let expected = if string(props, "LineStyle", "-")? == "none" {
        0.75
    } else {
        1.5
    };
    Ok(props.contains_key("LineWidth")
        && scalar(props, "LineWidth", f64::NAN)? == exported
        && exported == expected)
}
fn children<'a>(node: &'a Fields<'a>) -> Result<&'a [Fields<'a>]> {
    let value = field(node, "children")?;
    if matches!(value, Value::Number { count: 0, .. }) {
        return Ok(&[]);
    }
    value
        .structs()
        .ok_or_else(|| "Unsupported FIG children representation".into())
}
// Saved appearance only. This does not establish Bar HG reconstruction rules.
fn saved_edge(value: &Value<'_>) -> Option<Option<[u8; 3]>> {
    match value.text() {
        Some("none") => Some(None),
        Some(_) => None,
        None => color(value).ok().map(Some),
    }
}
fn bar_edge(node: &Fields<'_>, properties: &Fields<'_>) -> Option<[u8; 3]> {
    if let Some(edge) = properties.get("EdgeColor").and_then(saved_edge) {
        return edge;
    }
    // Preserve prior admission when modes/children cannot be represented. No
    // per-face color array is mapped to the fill; every saved face must agree.
    let children = children(node).ok()?;
    if children.len() > 64 {
        return None;
    }
    let mut found: Option<Option<[u8; 3]>> = None;
    for child in children {
        if string(child, "type", "").ok() != Some("patch") {
            continue;
        }
        let p = object(field(child, "properties").ok()?).ok()?;
        let edge = saved_edge(p.get("EdgeColor")?)?;
        if found.is_some_and(|old| old != edge) {
            return None;
        }
        found = Some(edge);
    }
    found.flatten()
}
fn saved_bar_pattern(properties: &Fields<'_>) -> LinePattern {
    match string(properties, "LineStyle", "-").unwrap_or("-") {
        "--" => LinePattern::Dashed,
        ":" => LinePattern::Dotted,
        "-." => LinePattern::DashDot,
        "none" => LinePattern::None,
        _ => LinePattern::Solid,
    }
}
fn limits(props: &Fields<'_>, key: &str) -> Result<Option<[f64; 2]>> {
    props
        .get(key)
        .map(|v| {
            if v.len() != 2 {
                return Err(format!("Invalid FIG {key}"));
            }
            Ok([
                v.number(0).ok_or("Invalid limit")?,
                v.number(1).ok_or("Invalid limit")?,
            ])
        })
        .transpose()
}
fn color(value: &Value<'_>) -> Result<[u8; 3]> {
    if value.len() != 3 {
        return Err("FIG requires a uniform RGB color".into());
    }
    let mut out = [0; 3];
    for (i, c) in out.iter_mut().enumerate() {
        let v = value.number(i).ok_or("Invalid FIG color")?;
        if !v.is_finite() || !(0. ..=1.).contains(&v) {
            return Err("Invalid FIG color".into());
        }
        *c = (v * 255.).round() as u8;
    }
    Ok(out)
}

pub fn load_matlab_figure(path: &Path) -> Result<Figure> {
    let bytes = super::read_bounded(path, mat::LIMIT)?;
    decode_matlab_figure(&bytes)
}
pub(super) fn decode_matlab_figure(bytes: &[u8]) -> Result<Figure> {
    let expanded = mat::expand(bytes)?;
    let value = expanded.hierarchy()?;
    let root = object(&value)?;
    if string(root, "type", "")? != "figure" {
        return Err("FIG must contain one figure".into());
    }
    if !scalar(root, "handle", f64::NAN)?.is_finite() {
        return Err("Invalid FIG figure handle".into());
    }
    let mut axes_nodes = Vec::new();
    let mut legend_nodes = Vec::new();
    let mut annotation_nodes = Vec::new();
    for child in children(root)? {
        match string(child, "type", "")? {
            "axes" => axes_nodes.push(child),
            "scribe.legend" | "legend" => legend_nodes.push(child),
            "scribe.scribeoverlay" => {
                for ann in children(child)? {
                    annotation_nodes.push(ann);
                }
            }
            "scribe.line" | "scribe.arrow" | "scribe.doublearrow" | "scribe.textarrow"
            | "scribe.rectangle" | "scribe.ellipse" => annotation_nodes.push(child),
            other => return Err(format!("Unsupported FIG graphics object: {other}")),
        }
    }
    if axes_nodes.is_empty() || axes_nodes.len() > 16 || legend_nodes.len() > axes_nodes.len() {
        return Err("FIG requires 1..16 axes and at most one legend per axes".into());
    }
    let positioned = axes_nodes.len() > 1
        || axes_nodes.iter().any(|a| {
            a.get("properties")
                .and_then(|v| v.structs())
                .and_then(|s| s.first())
                .is_some_and(|p| p.contains_key("Position"))
        });
    let mut positions = Vec::new();
    let mut handles = Vec::new();
    for axes in &axes_nodes {
        let handle = scalar(axes, "handle", f64::NAN)?;
        if !handle.is_finite() || handles.contains(&handle) {
            return Err("Invalid or duplicate FIG axes handle".into());
        }
        handles.push(handle);
        if positioned {
            let props = object(field(axes, "properties")?)?;
            if string(props, "Units", "normalized")? != "normalized" {
                return Err("Positioned FIG axes currently require normalized Units".into());
            }
            let p = field(props, "Position")?;
            if p.len() != 4 {
                return Err("FIG axes Position must contain four numbers".into());
            }
            positions.push(crate::plot::Position::from_normalized([
                p.number(0).ok_or("Invalid FIG Position")?,
                p.number(1).ok_or("Invalid FIG Position")?,
                p.number(2).ok_or("Invalid FIG Position")?,
                p.number(3).ok_or("Invalid FIG Position")?,
            ])?);
        }
    }
    let mut legends: Vec<Option<&Fields<'_>>> = vec![None; axes_nodes.len()];
    let mut legend_memberships = vec![None; axes_nodes.len()];
    let mut series_handles = None;
    for legend in legend_nodes {
        let p = object(field(legend, "properties")?)?;
        // An explicit peer must be valid even when only one axes exists.
        // Older single-axes hierarchies may omit it; absence is not a license
        // to ignore an explicitly stale, malformed or nonfinite reference.
        let index = if p.contains_key("Axes") {
            let peer = scalar(p, "Axes", f64::NAN)?;
            if !peer.is_finite() {
                return Err("FIG legend Axes peer must be finite".into());
            }
            handles
                .iter()
                .position(|h| *h == peer)
                .ok_or("FIG legend Axes peer does not identify a supported axes")?
        } else if axes_nodes.len() == 1 {
            0
        } else {
            return Err("FIG legend Axes peer is required for multiple axes".into());
        };
        // Build the bounded figure-local identity map only for explicit lists.
        // Legacy absent metadata retains its original all-series fallback.
        if p.get("ApplicationData").and_then(Value::structs).is_some() {
            if series_handles.is_none() {
                series_handles = Some(legend_series_handles(&axes_nodes)?);
            }
            legend_memberships[index] =
                validate_legend_membership(p, series_handles.as_ref().unwrap(), index)?;
        }
        if legends[index].replace(legend).is_some() {
            return Err("Multiple FIG legends for one axes are unsupported".into());
        }
    }
    let active = if positioned {
        let props = object(field(root, "properties")?)?;
        let handle = scalar(props, "CurrentAxes", handles[0])?;
        handles
            .iter()
            .position(|h| *h == handle)
            .ok_or("FIG CurrentAxes handle is missing")?
    } else {
        0
    };
    let mut figure = Figure {
        appearance: None,
        axes: crate::plot::Axes::default(),
        ..Figure::default()
    };
    let props = object(field(root, "properties")?)?;
    let name = string(props, "Name", "")?;
    if name.len() > 4096 {
        return Err("FIG Name exceeds 4096 UTF-8 bytes".into());
    }
    figure.name = name.to_owned();
    figure.invert_hardcopy = props
        .get("InvertHardcopy")
        .map(|value| match value.text() {
            Some("on") => Ok(true),
            Some("off") => Ok(false),
            _ => Err("FIG InvertHardcopy must be on or off".to_owned()),
        })
        .transpose()?;
    figure.color = props
        .get("Color")
        .map(color)
        .transpose()?
        .unwrap_or([255; 3]);
    figure.visible = match string(props, "Visible", "on")? {
        "on" => true,
        "off" => false,
        _ => return Err("FIG Visible must be on or off".into()),
    };
    figure.units = crate::plot::FigureUnits::parse(string(props, "Units", "pixels")?)?;
    if let Some(pos) = props.get("Position") {
        if pos.len() != 4 {
            return Err("FIG figure Position must contain four numbers".into());
        }
        let raw = [
            pos.number(0).ok_or("Invalid FIG figure Position")?,
            pos.number(1).ok_or("Invalid FIG figure Position")?,
            pos.number(2).ok_or("Invalid FIG figure Position")?,
            pos.number(3).ok_or("Invalid FIG figure Position")?,
        ];
        figure.position = figure.units.rect_to_pixels(raw);
    }
    figure.resize = match string(props, "Resize", "on")? {
        "on" => true,
        "off" => false,
        _ => return Err("FIG Resize must be on or off".into()),
    };
    figure.number_title = match string(props, "NumberTitle", "on")? {
        "on" => true,
        "off" => false,
        _ => return Err("FIG NumberTitle must be on or off".into()),
    };
    figure.window_state = crate::plot::WindowState::parse(string(props, "WindowState", "normal")?)?;
    for ann_node in annotation_nodes {
        figure.add_annotation(decode_fig_annotation(ann_node)?)?;
    }
    if positioned {
        figure.layout = Some(crate::plot::Layout {
            others: positions
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != active)
                .map(|(_, p)| crate::plot::Panel {
                    position: *p,
                    axes: Default::default(),
                })
                .collect(),
            ..crate::plot::Layout::plain(positions[active])
        });
        figure.validate()?; // Reject invalid geometry before sample allocation.
        figure.layout.as_mut().unwrap().others.clear();
    }
    let mut points = 0;
    let mut series = 0;
    let mut bars = 0;
    for (i, axes) in axes_nodes.into_iter().enumerate() {
        let (
            visible,
            labels,
            location,
            legend_colors,
            legend_orientation,
            legend_box,
            legend_num_columns,
            legend_interpreter,
        ) = if let Some(legend) = legends[i] {
            let p = object(field(legend, "properties")?)?;
            let orientation =
                crate::plot::LegendOrientation::parse(string(p, "Orientation", "vertical")?)?;
            let box_style = crate::plot::LegendBox::parse(string(p, "Box", "on")?)?;
            let num_columns = if p.contains_key("NumColumns") {
                let cols = scalar(p, "NumColumns", 1.)?;
                if !cols.is_finite()
                    || cols.fract() != 0.
                    || cols < 1.
                    || cols > crate::plot::MAX_SERIES as f64
                {
                    return Err("FIG legend NumColumns must be an integer in 1..64".into());
                }
                Some(cols as usize)
            } else {
                None
            };
            let interpreter =
                crate::plot::text_markup::Interpreter::parse(string(p, "Interpreter", "tex")?)?;
            let colors = crate::plot::LegendColors {
                background: optional_axes_rgb(p, "Color")?,
                text: optional_axes_rgb(p, "TextColor")?,
                border: optional_axes_rgb(p, "EdgeColor")?,
            };
            colors.validate()?;
            let colors = (colors != crate::plot::LegendColors::default()).then_some(colors);
            let strings = field(p, "String")?;
            if matches!(strings, Value::Cell(values) if values.len() > crate::plot::MAX_SERIES) {
                return Err("FIG legend exceeds 64 labels".into());
            }
            let labels = match strings {
                Value::Text(s) => vec![s.clone()],
                Value::Cell(values) => values
                    .iter()
                    .map(|v| {
                        v.text()
                            .map(str::to_owned)
                            .ok_or_else(|| "Unsupported FIG legend labels".to_owned())
                    })
                    .collect::<Result<Vec<_>>>()?,
                _ => return Err("Unsupported FIG legend labels".into()),
            };
            (
                string(p, "Visible", "on")? == "on",
                Some(labels),
                crate::plot::LegendLocation::parse(string(p, "Location", "northeast")?)?,
                colors,
                orientation,
                box_style,
                num_columns,
                interpreter,
            )
        } else {
            (
                false,
                None,
                crate::plot::LegendLocation::default(),
                None,
                crate::plot::LegendOrientation::default(),
                crate::plot::LegendBox::default(),
                None,
                crate::plot::text_markup::Interpreter::default(),
            )
        };
        let mut decoded = decode_axes(
            axes,
            visible,
            labels,
            legend_memberships[i].take(),
            crate::plot::MAX_POINTS - points,
            crate::plot::MAX_SERIES - series,
            10000 - bars,
        )?;
        decoded.legend_location = location;
        decoded.extras.legend_orientation = legend_orientation;
        decoded.extras.legend_box = legend_box;
        decoded.extras.legend_num_columns = legend_num_columns;
        decoded.extras.legend_interpreter = legend_interpreter;
        if let Some(colors) = legend_colors {
            decoded.colors.get_or_insert_with(Default::default).legend = Some(colors);
        }
        let props = object(field(axes, "properties")?)?;
        let legacy_position = string(props, "ActivePositionProperty", "position")?;
        if !matches!(legacy_position, "position" | "outerposition") {
            return Err("Unsupported FIG ActivePositionProperty".into());
        }
        if location.is_outside()
            || props.contains_key("PositionConstraint")
            || legacy_position == "outerposition"
        {
            use crate::plot::{AxesPlacement, PositionConstraint};
            let rectangle = |name: &str| -> Result<[f64; 4]> {
                let value = field(props, name)?;
                if value.len() != 4 {
                    return Err(format!("FIG axes {name} must contain four numbers"));
                }
                let mut result = [0.; 4];
                for (i, v) in result.iter_mut().enumerate() {
                    *v = value
                        .number(i)
                        .ok_or_else(|| format!("Invalid FIG {name}"))?;
                }
                crate::plot::Position::from_normalized(result)?;
                Ok(result)
            };
            let inner = rectangle("Position")?;
            let outer = if props.contains_key("OuterPosition") {
                rectangle("OuterPosition")?
            } else {
                [0., 0., 1., 1.]
            };
            let modern = props
                .get("PositionConstraint")
                .map(|_| {
                    PositionConstraint::parse(string(props, "PositionConstraint", "innerposition")?)
                })
                .transpose()?;
            let legacy = if props.contains_key("ActivePositionProperty") {
                Some(match string(props, "ActivePositionProperty", "position")? {
                    "position" => PositionConstraint::InnerPosition,
                    "outerposition" => PositionConstraint::OuterPosition,
                    _ => return Err("Unsupported FIG ActivePositionProperty".into()),
                })
            } else {
                None
            };
            if modern.is_some() && legacy.is_some() && modern != legacy {
                return Err("Conflicting FIG position constraints".into());
            }
            decoded.placement = Some(AxesPlacement {
                requested_inner: inner,
                requested_outer: outer,
                constraint: modern
                    .or(legacy)
                    .unwrap_or(PositionConstraint::InnerPosition),
                imported_solved: true,
            });
        }
        points += decoded.point_count();
        series += decoded.series.len();
        bars += decoded
            .series
            .iter()
            .filter(|s| s.kind == PlotKind::Bar)
            .map(|s| s.points.len())
            .sum::<usize>();
        if i == active {
            figure.axes = decoded.axes;
        } else {
            figure
                .layout
                .as_mut()
                .unwrap()
                .others
                .push(crate::plot::Panel {
                    position: positions[i],
                    axes: decoded.axes,
                });
        }
    }
    if let Some(layout) = &mut figure.layout {
        layout.others.sort_by_key(|p| p.position);
    }
    figure.validate()?;
    Ok(figure)
}
type FigSeriesHandles = BTreeMap<u64, Option<(usize, usize)>>;
fn fig_handle_key(value: f64) -> Result<u64> {
    if !value.is_finite() {
        return Err("FIG legend series handles must be finite and unique".into());
    }
    // MATLAB numeric handle equality treats both signed zeros as equal.
    Ok(if value == 0. { 0 } else { value.to_bits() })
}
fn legend_series_handles(axes: &[&Fields<'_>]) -> Result<FigSeriesHandles> {
    let mut handles = BTreeMap::new();
    for axis in axes {
        let key = fig_handle_key(scalar(axis, "handle", f64::NAN)?)?;
        if handles.insert(key, None).is_some() {
            return Err("Invalid or duplicate FIG axes handle".into());
        }
    }
    let mut total = 0;
    for (axis_index, axis) in axes.iter().enumerate() {
        let mut series_index = 0;
        for child in children(axis)? {
            if !matches!(
                string(child, "type", "")?,
                "graph2d.lineseries"
                    | "line"
                    | "specgraph.scattergroup"
                    | "scatter"
                    | "specgraph.barseries"
                    | "bar"
            ) {
                continue;
            }
            total += 1;
            if total > crate::plot::MAX_SERIES {
                return Err("FIG legend series count exceeds limit".into());
            }
            let key = fig_handle_key(scalar(child, "handle", f64::NAN)?)?;
            if handles
                .insert(key, Some((axis_index, series_index)))
                .is_some()
            {
                return Err("FIG legend series handles must be finite and unique".into());
            }
            series_index += 1;
        }
    }
    Ok(handles)
}
fn validate_legend_membership(
    props: &Fields<'_>,
    handles: &FigSeriesHandles,
    target: usize,
) -> Result<Option<Vec<usize>>> {
    let Some(app) = props.get("ApplicationData").and_then(Value::structs) else {
        // No explicit list: retain legacy count/label fallback, not an inferred selection.
        return Ok(None);
    };
    if app.len() != 1 {
        return Err("FIG legend membership metadata must be scalar".into());
    }
    let selected = field(&app[0], "PlotChildren")?;
    if selected.len() == 0 || selected.len() > crate::plot::MAX_SERIES {
        return Err("FIG PlotChildren requires 1..64 selected series".into());
    }
    let mut seen = [false; crate::plot::MAX_SERIES];
    let mut indices = Vec::with_capacity(selected.len());
    for i in 0..selected.len() {
        let actual = selected
            .number(i)
            .ok_or("Invalid FIG PlotChildren handle")?;
        let key = fig_handle_key(actual)?;
        let Some(Some((axis, index))) = handles.get(&key) else {
            return Err("FIG PlotChildren must identify admitted series in its own axes".into());
        };
        if *axis != target {
            return Err("FIG PlotChildren must identify admitted series in its own axes".into());
        }
        if std::mem::replace(&mut seen[*index], true) {
            return Err("FIG PlotChildren contains duplicate series".into());
        }
        indices.push(*index);
    }
    Ok(Some(indices))
}

/// Read only the actual saved RGB. ColorMode and Theme are not inferred.
fn optional_axes_rgb(props: &Fields<'_>, key: &str) -> Result<Option<[f64; 3]>> {
    let Some(value) = props.get(key) else {
        return Ok(None);
    };
    if !matches!(value, Value::Number { count: 3, .. }) {
        return Err(format!(
            "FIG {key} requires three real RGB values; color strings are unsupported"
        ));
    }
    let rgb = [value.number(0), value.number(1), value.number(2)];
    let [Some(r), Some(g), Some(b)] = rgb else {
        return Err(format!("Unsupported FIG {key} numeric storage"));
    };
    let rgb = [r, g, b];
    crate::plot::AxesColors::validate_rgb(rgb)?;
    Ok(Some(rgb))
}

fn optional_grid_style(props: &Fields<'_>) -> Result<Option<crate::plot::AxesGridStyle>> {
    use crate::plot::{AxesGridStyle, ColorMode};
    let keys = [
        "GridColor",
        "GridAlpha",
        "XColorMode",
        "YColorMode",
        "GridColorMode",
        "GridAlphaMode",
    ];
    if !keys.iter().any(|key| props.contains_key(*key)) {
        return Ok(None);
    }
    if !keys.iter().all(|key| props.contains_key(*key)) {
        return Err("FIG grid styling requires explicit GridColor, GridAlpha, XColorMode, YColorMode, GridColorMode and GridAlphaMode; absent historical defaults are not inferred".into());
    }
    let mode = |key| match string(props, key, "")? {
        "auto" => Ok(ColorMode::Auto),
        "manual" => Ok(ColorMode::Manual),
        _ => Err("FIG grid modes require auto or manual".to_owned()),
    };
    let grid = AxesGridStyle {
        color: optional_axes_rgb(props, "GridColor")?.unwrap(),
        alpha: scalar(props, "GridAlpha", f64::NAN)?,
        x_mode: mode("XColorMode")?,
        y_mode: mode("YColorMode")?,
        color_mode: mode("GridColorMode")?,
        alpha_mode: mode("GridAlphaMode")?,
    };
    grid.validate()?;
    Ok(Some(grid))
}

fn decode_axes(
    axes: &Fields<'_>,
    legend: bool,
    legend_labels: Option<Vec<String>>,
    legend_membership: Option<Vec<usize>>,
    remaining_points: usize,
    remaining_series: usize,
    remaining_bars: usize,
) -> Result<Figure> {
    let props = object(field(axes, "properties")?)?;
    let view = if let Some(view) = props.get("View") {
        let az = view.number(0).ok_or("Invalid FIG View")?;
        let el = view.number(1).ok_or("Invalid FIG View")?;
        if !az.is_finite() || !el.is_finite() {
            return Err("FIG View angles must be finite".into());
        }
        [az, el]
    } else {
        crate::plot::view3d::VIEW_2D
    };
    for key in ["XDir", "YDir"] {
        if string(props, key, "normal")? != "normal" {
            return Err("Reversed FIG axes are not supported".into());
        }
    }
    let scale = |key| match string(props, key, "linear")? {
        "linear" => Ok(AxisScale::Linear),
        "log" => Ok(AxisScale::Log),
        _ => Err("Unsupported FIG axis scale".to_owned()),
    };
    let grid = |key| match string(props, key, "off")? {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err("FIG grid visibility must be on or off".to_owned()),
    };
    // Minor grids are modeled (default style); an explicit minor-grid style is
    // not, so it is never silently discarded, even if hidden.
    for key in [
        "MinorGridColor",
        "MinorGridAlpha",
        "MinorGridColorMode",
        "MinorGridAlphaMode",
    ] {
        if props.contains_key(key) {
            return Err(format!(
                "Explicit FIG {key} is not supported; minor-grid styling is not represented"
            ));
        }
    }
    let minor_grid = [
        grid("XMinorGrid")?,
        grid("YMinorGrid")?,
        grid("ZMinorGrid")?,
    ];
    // An absent Box keeps the historical outline (files written before Box was
    // stored; MATLAB omits its own default 'off').
    let box_outline = match string(props, "Box", "on")? {
        "on" => true,
        "off" => false,
        _ => return Err("FIG Box must be on or off".into()),
    };
    let mut colors = crate::plot::AxesColors {
        grid: optional_grid_style(props)?,
        background: optional_axes_rgb(props, "Color")?,
        x: optional_axes_rgb(props, "XColor")?,
        y: optional_axes_rgb(props, "YColor")?,
        z: optional_axes_rgb(props, "ZColor")?,
        ..Default::default()
    };
    let xgrid = grid("XGrid")?;
    let ygrid = grid("YGrid")?;
    let zgrid = grid("ZGrid")?;
    if colors.grid.is_none() && ((xgrid && colors.x.is_some()) || (ygrid && colors.y.is_some())) {
        return Err("FIG ruler RGB with visible grids requires complete explicit grid color/alpha/modes; historical defaults are not inferred".into());
    }
    colors.validate()?;
    let mut f = Figure {
        identity: crate::ObjectIdentity::default(),
        axes: crate::plot::Axes {
            xscale: scale("XScale")?,
            yscale: scale("YScale")?,
            xlim: limits(props, "XLim")?,
            ylim: limits(props, "YLim")?,
            zlim: limits(props, "ZLim")?,
            hold: string(props, "NextPlot", "replace")? == "add",
            xgrid,
            ygrid,
            zgrid,
            view,
            box_outline,
            minor_grid,
            legend,
            ..Default::default()
        },
        layout: None,
        textboxes: Vec::new(),
        annotations: Vec::new(),
        name: String::new(),
        color: [255; 3],
        appearance: None,
        invert_hardcopy: None,
        ..Figure::default()
    };
    decode_fig_axes_extras(props, &mut f.axes)?;
    let children = children(axes)?;
    let special = field(axes, "special")?;
    if !matches!(special, Value::Number { .. }) {
        return Err("Invalid FIG label references".into());
    }
    let mut text_indices = Vec::new();
    for (i, label) in [
        &mut f.axes.title,
        &mut f.axes.xlabel,
        &mut f.axes.ylabel,
        &mut f.axes.zlabel,
        &mut f.axes.extras.subtitle.string,
    ]
    .into_iter()
    .enumerate()
    {
        let index = special.number(i).unwrap_or(0.);
        if index == 0. {
            continue;
        }
        if !index.is_finite() || index.fract() != 0. || index < 1. || index > children.len() as f64
        {
            return Err("Invalid FIG label reference".into());
        }
        let index = index as usize - 1;
        let text = &children[index];
        if string(text, "type", "")? != "text" {
            return Err("Invalid FIG label object".into());
        }
        let p = object(field(text, "properties")?)?;
        let (text_val, text_source) = decode_fig_label_string(p)?;
        *label = text_val;
        let interpreter =
            crate::plot::text_markup::Interpreter::parse(string(p, "Interpreter", "tex")?)?;
        let rgb = optional_axes_rgb(p, "Color")?;
        text_indices.push(index);
        if i == 4 {
            f.axes.extras.subtitle.string_lines = text_source;
            f.axes.extras.subtitle.interpreter = interpreter;
            f.axes.extras.subtitle.color = rgb;
            f.axes.extras.subtitle.font_size = p
                .contains_key("FontSize")
                .then(|| scalar(p, "FontSize", 0.))
                .transpose()?;
            f.axes.extras.subtitle.bold = match string(p, "FontWeight", "normal")? {
                "normal" => false,
                "bold" => true,
                _ => return Err("FIG subtitle FontWeight must be normal or bold".into()),
            };
            continue;
        }
        f.axes.extras.label_lines[i] = text_source;
        f.axes.extras.label_interpreters[i] = interpreter;
        match i {
            0 => colors.title = rgb,
            1 => colors.xlabel = rgb,
            2 => colors.ylabel = rgb,
            _ => colors.z = colors.z.or(rgb),
        }
    }
    if colors != crate::plot::AxesColors::default() {
        f.axes.colors = Some(colors);
    }
    for (index, node) in children.iter().enumerate() {
        let kind = string(node, "type", "")?;
        if kind == "text" && text_indices.contains(&index) || kind == "specgraph.baseline" {
            continue;
        }
        if matches!(
            kind,
            "matlab.graphics.chart.decoration.ConstantLine" | "constantline"
        ) {
            let cl = decode_fig_constant_line(node)?;
            if f.axes.extras.constant_lines.len() >= crate::plot::MAX_CONSTANT_LINES {
                return Err("FIG axes exceeds 256 constant line objects".into());
            }
            f.axes.extras.constant_lines.push(cl);
            continue;
        }
        if kind == "rectangle" {
            let rect = decode_fig_rectangle(node)?;
            if f.axes.extras.rectangles.len() >= crate::plot::MAX_RECTANGLES {
                return Err("FIG axes exceeds 256 rectangle objects".into());
            }
            f.axes.extras.rectangles.push(rect);
            continue;
        }
        let plot_kind = match kind {
            "graph2d.lineseries" | "line" => PlotKind::Line,
            "specgraph.scattergroup" | "scatter" => PlotKind::Scatter,
            "specgraph.barseries" | "bar" => PlotKind::Bar,
            _ => return Err(format!("Unsupported FIG axes object: {kind}")),
        };
        let p = object(field(node, "properties")?)?;
        let visible = match string(p, "Visible", "on")? {
            "on" => true,
            "off" => false,
            _ => return Err("Invalid FIG visibility".into()),
        };
        let z_data = p
            .get("ZData")
            .filter(|v| !matches!(v, Value::Number { count: 0, .. }));
        let x = field(p, "XData")?;
        let y = field(p, "YData")?;
        if !matches!(x, Value::Number { .. }) || !matches!(y, Value::Number { .. }) {
            return Err("FIG coordinates must be real numeric arrays".into());
        }
        if x.len() != y.len() || x.len() + f.point_count() > remaining_points {
            return Err("FIG series dimensions or point budget exceeded".into());
        }
        if f.series.len() >= remaining_series
            || (plot_kind == PlotKind::Bar
                && x.len()
                    + f.series
                        .iter()
                        .filter(|s| s.kind == PlotKind::Bar)
                        .map(|s| s.points.len())
                        .sum::<usize>()
                    > remaining_bars)
        {
            return Err("FIG exceeds the figure-wide series or bar budget".into());
        }
        let points = (0..x.len())
            .map(|i| {
                Ok([
                    x.number(i).ok_or("Unsupported FIG XData")?,
                    y.number(i).ok_or("Unsupported FIG YData")?,
                ])
            })
            .collect::<Result<Vec<_>>>()?;
        let mut series = Series::new(points, plot_kind);
        if let Some(z) = z_data {
            if !matches!(z, Value::Number { .. }) || z.len() != x.len() {
                return Err("FIG ZData must match XData/YData length".into());
            }
            let zv = (0..z.len())
                .map(|i| {
                    z.number(i)
                        .ok_or_else(|| "Unsupported FIG ZData".to_owned())
                })
                .collect::<Result<Vec<_>>>()?;
            series.z = zv.into();
        }
        // This legacy importer does not reconstruct MATLAB BaseLine properties.
        // Preserve its established fill-only Bar appearance explicitly.
        series.bar_baseline_visible = false;
        if plot_kind != PlotKind::Bar {
            let width = scalar(p, "LineWidth", 0.5)?;
            crate::plot::validate_line_width(width)?;
            series.line_width = Some(width);
        }
        series.visible = visible;
        series.legend_hidden = legend_hidden_hint(p)?;
        series.label = string(p, "DisplayName", "")?.to_owned();
        series.color = COLORS[f.series.len() % COLORS.len()];
        series.explicit_color = true;
        if implicit_width_hint(p, plot_kind)? {
            series.line_width = None;
        }
        let hint = scatter_hint(p)?;
        if hint.is_some() && plot_kind != PlotKind::Line {
            return Err("MatRustGraphics scatter hint requires a line object".into());
        }
        if plot_kind == PlotKind::Line {
            if let Some(c) = p.get("Color") {
                series.color = color(c)?;
            }
            let style = string(p, "LineStyle", "-")?;
            series.pattern = match style {
                "-" => LinePattern::Solid,
                "none" => LinePattern::None,
                "--" => LinePattern::Dashed,
                ":" => LinePattern::Dotted,
                "-." => LinePattern::DashDot,
                _ => return Err("Unsupported FIG line style".into()),
            };
            let marker = string(p, "Marker", "none")?;
            if marker != "none" {
                let shape = crate::plot::MarkerShape::parse(marker)?;
                series.markers = true;
                series.marker_filled = shape == crate::plot::MarkerShape::Point;
                if !matches!(
                    shape,
                    crate::plot::MarkerShape::Circle | crate::plot::MarkerShape::Point
                ) || p.contains_key("MarkerSize")
                {
                    series.marker_shape = Some(shape);
                }
            }
            if p.contains_key("MarkerSize") {
                let size = scalar(p, "MarkerSize", 6.)?;
                crate::plot::validate_marker_size(size)?;
                series.marker_size = Some(size);
            }
            if let Some(pattern) = hint {
                if style != "none" || !series.markers {
                    return Err(
                        "MatRustGraphics scatter hint requires LineStyle none and a marker".into(),
                    );
                }
                series.kind = PlotKind::Scatter;
                series.pattern = pattern;
            }
            let face = string(p, "MarkerFaceColor", "none");
            if matches!(face, Ok("auto")) {
                series.marker_filled = true;
            } else if let Some(v) = p.get("MarkerFaceColor")
                && v.text().is_none()
            {
                if color(v)? != series.color {
                    return Err("Separate FIG marker/line colors are not supported".into());
                }
                series.marker_filled = true;
            }
            apply_scatter_style_hint(p, &mut series)?;
        } else if plot_kind == PlotKind::Scatter {
            series.markers = true;
            series.color = color(field(p, "CData")?)?;
            // Flat uses the uniform CData already admitted above. Explicit
            // RGB is representable only when it matches that series color.
            // Auto instead follows axes Color and cannot be silently mapped
            // to CData by this single-color model.
            series.marker_filled = match p.get("MarkerFaceColor") {
                None => false,
                Some(value) => match value.text() {
                    Some("none") => false,
                    Some("flat") => true,
                    Some(_) => return Err("FIG scatter MarkerFaceColor supports none, flat or matching RGB; automatic or other face colors are not represented".into()),
                    None => {
                        if color(value)? != series.color {
                            return Err("Separate FIG scatter face and series colors are not supported".into());
                        }
                        true
                    }
                },
            };
            let shape = crate::plot::MarkerShape::parse(string(p, "Marker", "o")?)?;
            series.marker_shape = Some(shape);
            series.marker_filled |= shape == crate::plot::MarkerShape::Point;
            if p.get("SizeData").is_some_and(|v| v.len() > 1) {
                return Err("Variable-size FIG scatter points are not supported".into());
            }
            let area = scalar(p, "SizeData", 36.)?;
            let size = area.sqrt();
            crate::plot::validate_marker_size(size)?;
            series.marker_size = Some(size);
        } else {
            series.face_edge_color = bar_edge(node, p);
            series.pattern = saved_bar_pattern(p);
            // Unknown/out-of-scope width retains prior admission, rather than
            // rejecting a file whose Bar appearance was formerly ignored.
            let edge_width = scalar(p, "LineWidth", 0.5).ok();
            series.line_width = edge_width.filter(|v| crate::plot::validate_line_width(*v).is_ok());
            if scalar(p, "BaseValue", 0.)? != 0. {
                return Err("Custom FIG bar baselines are not supported".into());
            }
            if string(p, "BarLayout", "grouped")? != "grouped"
                || string(p, "Horizontal", "off")? != "off"
            {
                return Err("Only vertical grouped FIG bars are supported".into());
            }
            let spacing = series
                .points
                .iter()
                .zip(series.points.iter().skip(1))
                .map(|(a, b)| b[0] - a[0])
                .reduce(f64::min)
                .unwrap_or(1.);
            if !spacing.is_finite() || spacing <= 0. {
                return Err("FIG bar coordinates must increase".into());
            }
            // MATLAB serializes WidthScaleFactor and XOffset in axis units.
            // BarWidth scales the rendered rectangle, not the group offset.
            series.bar_width =
                scalar(p, "WidthScaleFactor", spacing)? * scalar(p, "BarWidth", 0.8)?;
            series.bar_offset = scalar(p, "XOffset", 0.)?;
            if let Some(c) = p.get("FaceColor").filter(|v| v.text().is_none()) {
                series.color = color(c)?;
            } else {
                let palette = scalar(p, "FaceColorIndex", f.series.len() as f64 + 1.)?;
                if palette >= 1. {
                    series.color = COLORS[(palette as usize - 1) % COLORS.len()];
                }
            }
        }
        f.add(vec![series])?;
    }
    if let Some(labels) = legend_labels {
        let indices = legend_membership.as_deref();
        let count = indices.map_or(f.series.len(), <[usize]>::len);
        if labels.len() != count {
            return Err("FIG legend label count must match its selected series".into());
        }
        for (position, label) in labels.into_iter().enumerate() {
            let index = indices.map_or(position, |members| members[position]);
            let series = f
                .series
                .get_mut(index)
                .ok_or("Invalid FIG legend series index")?;
            // The model uses one label for DisplayName and legend String. Do not
            // silently erase a distinct nonempty source DisplayName.
            if !series.label.is_empty() && series.label != label {
                return Err("FIG legend label differs from selected series DisplayName".into());
            }
            series.label = label;
        }
        if let Some(indices) = legend_membership {
            f.legend_state = Some(crate::plot::LegendState {
                identity: crate::ObjectIdentity::default(),
                members: indices
                    .into_iter()
                    .map(|i| f.series[i].identity.clone())
                    .collect(),
                // Capture the saved list. This is a deliberate import policy,
                // not an inferred omitted MATLAB AutoUpdate default.
                auto_update: false,
            });
        }
    }
    crate::engine::plot_properties::refresh_scatter_clim(&mut f.axes);
    f.validate()?;
    Ok(f)
}

fn num(values: &[f64]) -> mat::Encoded<'static> {
    mat::numbers(values.iter().copied())
}
fn node<'a>(
    kind: &str,
    handle: f64,
    props: Vec<(&str, mat::Encoded<'a>)>,
    children: Vec<Vec<(&str, mat::Encoded<'a>)>>,
    special: &[f64],
) -> Vec<(&'static str, mat::Encoded<'a>)> {
    vec![
        ("type", mat::text(kind)),
        ("handle", num(&[handle])),
        ("properties", mat::structure("", props)),
        ("children", mat::structures("", children)),
        ("special", num(special)),
    ]
}
pub fn save_matlab_figure(path: &Path, figure: &Figure) -> Result<()> {
    save_matlab_figure_with_cancel(path, figure, &std::sync::atomic::AtomicBool::new(false))
}
/// Explicit FIG export also permits filenames whose extensions differ from FIG.
pub fn save_matlab_figure_with_cancel(
    path: &Path,
    figure: &Figure,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    save_matlab_figure_with_theme(path, figure, crate::plot::BaseTheme::Light, cancel)
}
/// FIG stores a fixed RGB appearance, not automatic theme intent. Use MFIG
/// for editable host-following intent. FIG serializes captured RGB, not Theme.
pub fn save_matlab_figure_with_theme(
    path: &Path,
    figure: &Figure,
    host: crate::plot::BaseTheme,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    let check = || {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            Err("Cancelled".to_owned())
        } else {
            Ok(())
        }
    };
    check()?;
    let bytes = encode_matlab_figure_with_theme(figure, host)?;
    #[cfg(test)]
    static_legend_writer_tests::before_publish(cancel);
    check()?;
    super::atomic_write_with_cancel(path, &bytes, cancel)
}
/// Returns the first unsupported FIG feature using only bounded plot metadata.
/// This does not validate sample data or guarantee that a later save succeeds.
/// Use `.mrfig` to preserve features that the current MATLAB FIG writer rejects.
pub fn matlab_figure_export_restriction(figure: &Figure) -> Option<&'static str> {
    matlab_figure_export_restriction_with_theme(figure, crate::plot::BaseTheme::Light)
}
pub fn matlab_figure_export_restriction_with_theme(
    figure: &Figure,
    _host: crate::plot::BaseTheme,
) -> Option<&'static str> {
    if figure
        .all_axes()
        .any(|(_, axes)| !axes.extras.volumes.is_empty())
    {
        return Some("volshow objects are saved only in .mrfig");
    }
    if figure.empty_layout() {
        return Some("MATLAB FIG export requires at least one axes");
    }
    if !figure.textboxes.is_empty()
        || figure
            .all_axes()
            .any(|(_, a)| a.series.iter().any(|s| s.kind == PlotKind::Patch))
    {
        return Some("MATLAB FIG patch/textbox writing is not verified; use MFIG or a plot script");
    }
    if figure.layout.as_ref().is_some_and(|l| !l.title.is_empty()) {
        return Some(
            "MATLAB FIG export of shared titles is not verified; use MFIG to preserve the complete figure",
        );
    }
    for (_, axes) in figure.all_axes() {
        if !axes.baselines.is_empty() {
            return Some("Explicit baseline FIG reconstruction is not verified; use MFIG");
        }
        if (axes.legend_state.is_some()
            || axes.legend
            || !axes.legend_location.is_default()
            || axes.colors.is_some_and(|c| c.legend.is_some()))
            && !static_legend_supported(figure, axes)
        {
            return Some(
                "MATLAB FIG legends require one axes, 1..64 labeled Line series in original order, visible northeast, and AutoUpdate off; use .mrfig for other legend states",
            );
        }
        if axes.placement.is_some() {
            return Some(
                "MATLAB FIG export of explicit inner/outer axes placement is not verified yet; save as .mrfig or export a MATLAB script to preserve the layout request",
            );
        }
        for series in &axes.series {
            if matches!(series.kind, PlotKind::Stairs | PlotKind::Stem) {
                return Some(
                    "MATLAB FIG export of stairs/stem is not verified yet; use editable MFIG or SVG/PNG/PDF export",
                );
            }
            if series.kind == PlotKind::Bar {
                return Some(
                    "MATLAB FIG bar export is not supported yet; save as .mrfig to preserve the complete figure",
                );
            }
        }
    }
    None
}
// Narrow declarative HG subset. This does not claim MATLAB restoration parity.
fn static_legend_supported(figure: &Figure, axes: &crate::plot::Axes) -> bool {
    figure.layout.is_none()
        && axes.legend
        && axes.legend_location.is_default()
        && axes
            .legend_state
            .as_ref()
            .is_some_and(|state| !state.auto_update)
        && !axes.series.is_empty()
        && axes.series.len() <= crate::plot::MAX_SERIES
        && axes.series.iter().all(|series| {
            series.kind == PlotKind::Line && !series.label.is_empty() && series.label.len() <= 4096
        })
        && axes
            .legend_members()
            .map(|(index, _)| index)
            .eq(0..axes.series.len())
}
struct LegendEncoding {
    handle: f64,
    members: Vec<f64>,
}
fn encode_static_legend<'a>(
    axes: &'a crate::plot::Axes,
    plan: &LegendEncoding,
    appearance: crate::plot::ResolvedAppearance,
) -> Result<Vec<(&'static str, mat::Encoded<'a>)>> {
    if plan.members.len() != axes.series.len() {
        return Err("Legend handle allocation does not match its series".into());
    }
    let labels: Vec<_> = axes.series.iter().map(|s| s.label.as_str()).collect();
    let mut properties = vec![
        ("Axes", num(&[2.])),
        ("String", mat::legend_text_cells(&labels)?),
        ("Location", mat::text("northeast")),
        ("Visible", mat::text("on")),
        (
            "Interpreter",
            mat::text(axes.extras.legend_interpreter.as_str()),
        ),
        (
            "Orientation",
            mat::text(axes.extras.legend_orientation.name()),
        ),
        ("Box", mat::text(axes.extras.legend_box.name())),
        (
            "ApplicationData",
            mat::structure(
                "",
                vec![("PlotChildren", mat::numbers(plan.members.iter().copied()))],
            ),
        ),
    ];
    if let Some(cols) = axes.extras.legend_num_columns {
        properties.push(("NumColumns", num(&[cols as f64])));
    }
    let colors = axes.legend_colors();
    let resolved = colors.resolved(appearance);
    for (name, supplied, fallback) in [
        ("Color", colors.background, resolved[0]),
        ("TextColor", colors.text, resolved[1]),
        ("EdgeColor", colors.border, resolved[2]),
    ] {
        if let Some(rgb) =
            supplied.or((appearance.theme == crate::plot::BaseTheme::Dark).then_some(fallback))
        {
            properties.push((name, num(&rgb)));
        }
    }
    Ok(node("scribe.legend", plan.handle, properties, vec![], &[]))
}
#[cfg(test)]
pub(super) fn encode_matlab_figure(figure: &Figure) -> Result<Vec<u8>> {
    encode_matlab_figure_with_theme(figure, crate::plot::BaseTheme::Light)
}
pub(super) fn encode_matlab_figure_with_theme(
    figure: &Figure,
    host: crate::plot::BaseTheme,
) -> Result<Vec<u8>> {
    figure.validate()?;
    if let Some(reason) = matlab_figure_export_restriction_with_theme(figure, host) {
        return Err(reason.into());
    }
    let appearance = figure.resolve_appearance(host);
    let mut panels: Vec<_> = figure.all_axes().collect();
    panels.sort_by_key(|(p, _)| *p);
    let scale = panels.iter().fold(1_f64, |s, (p, _)| {
        let [l, t, r, b] = p.rectangle();
        s.min(r - l).min(b - t)
    });
    let mut next_handle = if figure.layout.is_some() { 100. } else { 10. };
    // This branch admits one axes only. Root=1, axes=2 and legend=3 are
    // reserved before series/text allocation begins at 10; no guessed members.
    let mut legend = figure.legend.then(|| LegendEncoding {
        handle: 3.,
        members: Vec::with_capacity(figure.series.len()),
    });
    let mut root_children = Vec::with_capacity(panels.len() + usize::from(legend.is_some()));
    let mut current = 0;
    for (i, (position, axes)) in panels.into_iter().enumerate() {
        let handle = i as f64 + 2.;
        if position == figure.active_position() {
            current = i;
        }
        let normalized = if figure.layout.is_some() {
            Some(position.normalized_plot_box().unwrap_or_else(|| {
                let [l, t, r, b] = position.rectangle();
                [
                    l + 70. * scale / 960.,
                    1. - b + 70. * scale / 560.,
                    r - l - 110. * scale / 960.,
                    b - t - 130. * scale / 560.,
                ]
            }))
        } else {
            None
        };
        root_children.push(encode_axes(
            axes,
            handle,
            &mut next_handle,
            normalized,
            appearance,
            legend.as_mut(),
        )?);
    }
    if let Some(legend) = &legend {
        root_children.push(encode_static_legend(&figure.axes, legend, appearance)?);
    }
    if !figure.annotations.is_empty() {
        let mut ann_children = Vec::with_capacity(figure.annotations.len());
        for ann in &figure.annotations {
            ann_children.push(encode_fig_annotation(ann, next_handle)?);
            next_handle += 1.;
        }
        let overlay_handle = next_handle;
        root_children.push(node(
            "scribe.scribeoverlay",
            overlay_handle,
            vec![("Visible", mat::text("on"))],
            ann_children,
            &[],
        ));
    }
    let mut props = vec![
        (
            "Visible",
            mat::text(if figure.visible { "on" } else { "off" }),
        ),
        ("Units", mat::text(figure.units.as_str())),
        (
            "Position",
            num(&figure.units.rect_from_pixels(figure.position)),
        ),
        (
            "Resize",
            mat::text(if figure.resize { "on" } else { "off" }),
        ),
        (
            "NumberTitle",
            mat::text(if figure.number_title { "on" } else { "off" }),
        ),
        ("WindowState", mat::text(figure.window_state.as_str())),
        (
            "Color",
            num(&appearance.canvas.map(|c| f64::from(c) / 255.)),
        ),
    ];
    if let Some(invert) = figure
        .invert_hardcopy
        .or((appearance.theme == crate::plot::BaseTheme::Dark).then_some(false))
    {
        props.push((
            "InvertHardcopy",
            mat::text(if invert { "on" } else { "off" }),
        ));
    }
    if !figure.name.is_empty() {
        props.push(("Name", mat::text(&figure.name)));
    }
    // Keep the selected axes first for MatRust reopening. A live CurrentAxes
    // property must not be assigned before the child graphics objects exist.
    root_children.rotate_left(current);
    let root = node("figure", 1., props, root_children, &[]);
    mat::file(mat::structure("hgS_070000", root))
}
// Project an explicit, bounded grid record only for this serialized view.
// Legacy unstyled scenes keep their existing bytes. Historical import still
// rejects incomplete records; this is a snapshot of known MatRust policy.
fn fig_grid_projection(
    axes: &crate::plot::Axes,
    appearance: crate::plot::ResolvedAppearance,
) -> Option<crate::plot::AxesGridStyle> {
    let colors = axes.colors.unwrap_or_default();
    colors.grid.or_else(|| {
        (!appearance.legacy
            || (axes.xgrid && colors.x.is_some())
            || (axes.ygrid && colors.y.is_some()))
        .then(|| {
            let mut grid =
                crate::plot::AxesGridStyle::from_appearance(appearance, !appearance.legacy);
            if appearance.theme == crate::plot::BaseTheme::Dark {
                // Freeze only implicit product policy; supplied mode records remain exact.
                grid.color_mode = crate::plot::ColorMode::Manual;
                grid.alpha_mode = crate::plot::ColorMode::Manual;
            }
            grid
        })
    })
}
fn encode_axes<'a>(
    figure: &'a crate::plot::Axes,
    handle: f64,
    next_handle: &mut f64,
    position: Option<[f64; 4]>,
    appearance: crate::plot::ResolvedAppearance,
    mut legend: Option<&mut LegendEncoding>,
) -> Result<Vec<(&'static str, mat::Encoded<'a>)>> {
    let mut children = Vec::new();
    for s in &figure.series {
        let style = if s.kind == PlotKind::Scatter {
            "none"
        } else {
            match s.pattern {
                LinePattern::Solid => "-",
                LinePattern::None => "none",
                LinePattern::Dashed => "--",
                LinePattern::Dotted => ":",
                LinePattern::DashDot => "-.",
            }
        };
        let marker = if s.kind == PlotKind::Scatter || s.markers {
            s.effective_marker().symbol()
        } else {
            "none"
        };
        let public_width = s
            .line_width
            .unwrap_or(if style == "none" { 0.75 } else { 1.5 });
        // Capability preflight excludes Patch, whose coordinates may be shared
        // views. Admitted HG series always retain their original dense samples.
        let samples = s
            .points
            .dense()
            .ok_or("FIG export requires dense supported series")?;
        let mut props = vec![
            ("XData", mat::coordinate(samples, 0)),
            ("YData", mat::coordinate(samples, 1)),
            (
                "Color",
                num(&appearance.series_color(s).map(|v| v as f64 / 255.)),
            ),
            ("LineStyle", mat::text(style)),
            ("LineWidth", num(&[public_width])),
            ("Marker", mat::text(marker)),
            ("DisplayName", mat::text(&s.label)),
            ("Visible", mat::text(if s.visible { "on" } else { "off" })),
        ];
        if !s.z.is_empty() {
            props.push(("ZData", mat::numbers(s.z.iter().copied())));
        }
        let mut userdata = Vec::new();
        if s.line_width.is_none() {
            userdata.push((
                "MatRustImplicitWidth",
                mat::structure(
                    "",
                    vec![
                        ("Version", num(&[1.])),
                        ("ExportedValue", num(&[public_width])),
                    ],
                ),
            ));
        }
        if s.kind == PlotKind::Scatter {
            let pattern = match s.pattern {
                LinePattern::Solid => "Solid",
                LinePattern::Dashed => "Dashed",
                LinePattern::Dotted => "Dotted",
                LinePattern::DashDot => "DashDot",
                LinePattern::None => "None",
            };
            userdata.push((
                "MatRustGraphics",
                mat::structure(
                    "",
                    vec![
                        ("Version", num(&[1.])),
                        ("Kind", mat::text("scatter")),
                        ("Pattern", mat::text(pattern)),
                    ],
                ),
            ));
        }
        if s.kind == PlotKind::Scatter && s.has_scatter_style() {
            let mut fields = vec![
                ("Version", num(&[1.])),
                ("MarkerFaceColor", encode_marker_color(s.marker_face_color)),
                ("MarkerEdgeColor", encode_marker_color(s.marker_edge_color)),
            ];
            if let Some(sizes) = &s.marker_sizes {
                fields.push(("SizeData", mat::numbers(sizes.iter().copied())));
            }
            match &s.scatter_cdata {
                Some(crate::plot::ScatterCData::Rgb(rgbs)) => {
                    fields.push((
                        "CDataRgb",
                        mat::numbers(rgbs.as_flattened().iter().map(|&v| f64::from(v) / 255.)),
                    ));
                }
                Some(crate::plot::ScatterCData::Indexed(vals)) => {
                    fields.push(("CDataIndexed", mat::numbers(vals.iter().copied())));
                }
                None => {}
            }
            if let Some(a) = s.marker_face_alpha {
                fields.push(("MarkerFaceAlpha", num(&[a])));
            }
            if let Some(a) = s.marker_edge_alpha {
                fields.push(("MarkerEdgeAlpha", num(&[a])));
            }
            userdata.push(("MatRustScatter", mat::structure("", fields)));
        }
        if s.legend_hidden {
            userdata.push((
                "MatRustLegend",
                mat::structure(
                    "",
                    vec![
                        ("Version", num(&[1.])),
                        ("IconDisplayStyle", mat::text("off")),
                    ],
                ),
            ));
        }
        if !userdata.is_empty() {
            props.push(("UserData", mat::structure("", userdata)));
        }
        if s.has_marker_style() {
            props.push(("MarkerSize", num(&[s.marker_size_points()])));
            props.push((
                "MarkerFaceColor",
                if s.marker_is_filled() {
                    // Public Line MarkerFaceColor auto is background-dependent,
                    // not the series color. Emit the actual uniform RGB fill.
                    num(&appearance.series_color(s).map(|v| f64::from(v) / 255.))
                } else {
                    mat::text("none")
                },
            ));
        }
        if let Some(plan) = &mut legend {
            plan.members.push(*next_handle);
        }
        children.push(node("graph2d.lineseries", *next_handle, props, vec![], &[]));
        *next_handle += 1.;
    }
    for cl in &figure.extras.constant_lines {
        children.push(encode_fig_constant_line(cl, *next_handle));
        *next_handle += 1.;
    }
    for rect in &figure.extras.rectangles {
        children.push(encode_fig_rectangle(rect, *next_handle));
        *next_handle += 1.;
    }
    let mut special = vec![0.; 4];
    let mut colors = figure.colors.unwrap_or_default();
    let grid = fig_grid_projection(figure, appearance);
    if appearance.theme == crate::plot::BaseTheme::Dark {
        let normalized = |rgb: [u8; 3]| rgb.map(|v| f64::from(v) / 255.);
        // Local static projection; retain every supplied normalized RGB bit.
        colors.title.get_or_insert(normalized(appearance.text));
        colors
            .xlabel
            .get_or_insert(colors.x.unwrap_or(normalized(appearance.text)));
        colors
            .ylabel
            .get_or_insert(colors.y.unwrap_or(normalized(appearance.text)));
        colors.background.get_or_insert(normalized(appearance.axes));
        colors.x.get_or_insert(normalized(appearance.axis));
        colors.y.get_or_insert(normalized(appearance.axis));
    }
    // Light Auto ruler modes keep absent RGB. Dark snapshots above are explicit,
    // with their effective label colors captured before introducing ruler RGB.
    let label_colors = [colors.title, colors.xlabel, colors.ylabel, colors.z];
    for (i, label) in [
        &figure.title,
        &figure.xlabel,
        &figure.ylabel,
        &figure.zlabel,
    ]
    .into_iter()
    .enumerate()
    {
        if !label.is_empty() || label_colors[i].is_some() || figure.extras.label_lines[i].is_some()
        {
            use crate::plot::text_markup::Interpreter;
            let source = figure.extras.label_lines[i].as_ref();
            let interpreter = figure.extras.label_interpreters[i];
            // Preserve the historical literal FIG bytes for plain scalar labels.
            let wire_interpreter = if interpreter == Interpreter::Tex
                && source.is_none()
                && !label.contains(['\\', '{', '}', '_', '^'])
            {
                Interpreter::None
            } else {
                interpreter
            };
            let mut text_properties = encode_fig_label_properties(label, source, wire_interpreter)?;
            if let Some(rgb) = label_colors[i] {
                text_properties.push(("Color", num(&rgb)));
            }
            children.push(node("text", *next_handle, text_properties, vec![], &[]));
            *next_handle += 1.;
            special[i] = children.len() as f64;
        }
    }
    let subtitle = &figure.extras.subtitle;
    if !subtitle.is_default() {
        let mut props = encode_fig_label_properties(
            &subtitle.string,
            subtitle.string_lines.as_ref(),
            subtitle.interpreter,
        )?;
        if let Some(rgb) = subtitle.color {
            props.push(("Color", num(&rgb)));
        }
        if let Some(size) = subtitle.font_size {
            props.push(("FontSize", num(&[size])));
        }
        props.push((
            "FontWeight",
            mat::text(if subtitle.bold { "bold" } else { "normal" }),
        ));
        children.push(node("text", *next_handle, props, vec![], &[]));
        *next_handle += 1.;
        // Subtitle is an optional fifth label, following title and X/Y/Z labels.
        special.push(children.len() as f64);
    }
    let (x, y) = figure.bounds();
    let mut props = vec![
        ("XLim", num(&x)),
        ("YLim", num(&y)),
        (
            "XScale",
            mat::text(if figure.xscale == AxisScale::Log {
                "log"
            } else {
                "linear"
            }),
        ),
        (
            "YScale",
            mat::text(if figure.yscale == AxisScale::Log {
                "log"
            } else {
                "linear"
            }),
        ),
        ("XGrid", mat::text(if figure.xgrid { "on" } else { "off" })),
        ("YGrid", mat::text(if figure.ygrid { "on" } else { "off" })),
        (
            "NextPlot",
            mat::text(if figure.hold { "add" } else { "replace" }),
        ),
        ("View", num(&figure.view)),
    ];
    // Grid commands set ZGrid in 2-D too. Only write the Z grid alongside
    // actual Z-axis data or configuration, so it cannot imply a new ZLim.
    if figure.is_3d() || figure.zlim.is_some() || !figure.zlabel.is_empty() || figure.minor_grid[2]
    {
        let z_bounds = figure.zlim.unwrap_or_else(|| {
            let mut min_z = f64::INFINITY;
            let mut max_z = f64::NEG_INFINITY;
            for s in &figure.series {
                for &zv in s.z.iter() {
                    if zv.is_finite() {
                        min_z = min_z.min(zv);
                        max_z = max_z.max(zv);
                    }
                }
            }
            if min_z.is_finite() && max_z.is_finite() {
                if min_z < max_z {
                    [min_z, max_z]
                } else {
                    [min_z - 1., max_z + 1.]
                }
            } else {
                [0., 1.]
            }
        });
        props.push(("ZLim", num(&z_bounds)));
        props.push(("ZGrid", mat::text(if figure.zgrid { "on" } else { "off" })));
        if figure.minor_grid[2] {
            props.push(("ZMinorGrid", mat::text("on")));
        }
    }
    for (property, rgb) in [
        ("Color", colors.background),
        ("XColor", colors.x),
        ("YColor", colors.y),
        ("ZColor", colors.z),
    ] {
        if let Some(rgb) = rgb {
            props.push((property, num(&rgb)));
        }
    }
    encode_fig_axes_extras(figure, &mut props)?;
    // Only non-default states, so default files keep their historical bytes;
    // the reader takes an absent Box as on and absent minor grids as off.
    for (property, on) in [
        ("XMinorGrid", figure.minor_grid[0]),
        ("YMinorGrid", figure.minor_grid[1]),
    ] {
        if on {
            props.push((property, mat::text("on")));
        }
    }
    if !figure.box_outline {
        props.push(("Box", mat::text("off")));
    }
    if let Some(position) = position {
        props.push(("Units", mat::text("normalized")));
        props.push(("Position", num(&position)));
        props.push(("ActivePositionProperty", mat::text("position")));
    }
    if let Some(grid) = grid {
        // Assign numeric values before modes: assigning Color/Alpha can change
        // their modes in HG. The complete mode quartet must be restored last.
        props.push(("GridColor", num(&grid.color)));
        props.push(("GridAlpha", num(&[grid.alpha])));
        props.push(("XColorMode", mat::text(grid.x_mode.as_str())));
        props.push(("YColorMode", mat::text(grid.y_mode.as_str())));
        props.push(("GridColorMode", mat::text(grid.color_mode.as_str())));
        props.push(("GridAlphaMode", mat::text(grid.alpha_mode.as_str())));
    }
    if let Some(plan) = legend {
        props.push(("LegendPeerHandle", num(&[plan.handle])));
    }
    Ok(node("axes", handle, props, children, &special))
}

fn line_pattern_str(pattern: LinePattern) -> &'static str {
    match pattern {
        LinePattern::Solid => "-",
        LinePattern::None => "none",
        LinePattern::Dashed => "--",
        LinePattern::Dotted => ":",
        LinePattern::DashDot => "-.",
    }
}

fn parse_line_pattern_str(style: &str) -> Result<LinePattern> {
    match style {
        "-" => Ok(LinePattern::Solid),
        "none" => Ok(LinePattern::None),
        "--" => Ok(LinePattern::Dashed),
        ":" => Ok(LinePattern::Dotted),
        "-." => Ok(LinePattern::DashDot),
        _ => Err("Unsupported FIG line style".into()),
    }
}

fn encode_fig_annotation(
    ann: &crate::plot::Annotation,
    handle: f64,
) -> Result<Vec<(&'static str, mat::Encoded<'static>)>> {
    use crate::plot::AnnotationKind;
    let kind = match ann.kind {
        AnnotationKind::Line => "scribe.line",
        AnnotationKind::Arrow => "scribe.arrow",
        AnnotationKind::DoubleArrow => "scribe.doublearrow",
        AnnotationKind::TextArrow => "scribe.textarrow",
        AnnotationKind::Rectangle => "scribe.rectangle",
        AnnotationKind::Ellipse => "scribe.ellipse",
    };
    let mut props = vec![
        ("Position", num(&ann.position)),
        ("Color", num(&ann.color.map(|c| f64::from(c) / 255.))),
        (
            "EdgeColor",
            if ann.edge_none {
                mat::text("none")
            } else {
                num(&ann.color.map(|c| f64::from(c) / 255.))
            },
        ),
        ("LineWidth", num(&[ann.line_width])),
        ("LineStyle", mat::text(line_pattern_str(ann.pattern))),
        ("HeadStyle", mat::text(ann.head_style.name())),
        ("HeadLength", num(&[ann.head_length])),
        ("HeadWidth", num(&[ann.head_width])),
        ("String", mat::text(&ann.text)),
        ("FontSize", num(&[ann.font_size])),
        (
            "FaceColor",
            ann.face_color.map_or_else(
                || mat::text("none"),
                |c| num(&c.map(|v| f64::from(v) / 255.)),
            ),
        ),
        ("FaceAlpha", num(&[ann.face_alpha])),
        ("Visible", mat::text(if ann.visible { "on" } else { "off" })),
    ];
    let _ = &mut props;
    Ok(node(kind, handle, props, vec![], &[]))
}

fn decode_fig_annotation(ann_node: &Fields<'_>) -> Result<crate::plot::Annotation> {
    use crate::plot::{Annotation, AnnotationHeadStyle, AnnotationKind};
    let kind = match string(ann_node, "type", "")? {
        "scribe.line" => AnnotationKind::Line,
        "scribe.arrow" => AnnotationKind::Arrow,
        "scribe.doublearrow" => AnnotationKind::DoubleArrow,
        "scribe.textarrow" => AnnotationKind::TextArrow,
        "scribe.rectangle" => AnnotationKind::Rectangle,
        "scribe.ellipse" => AnnotationKind::Ellipse,
        other => return Err(format!("Unsupported FIG annotation type: {other}")),
    };
    let p = object(field(ann_node, "properties")?)?;
    let pos_val = field(p, "Position")?;
    if pos_val.len() != 4 {
        return Err("FIG annotation Position must contain four numbers".into());
    }
    let position = [
        pos_val.number(0).ok_or("Invalid FIG annotation Position")?,
        pos_val.number(1).ok_or("Invalid FIG annotation Position")?,
        pos_val.number(2).ok_or("Invalid FIG annotation Position")?,
        pos_val.number(3).ok_or("Invalid FIG annotation Position")?,
    ];
    let mut ann = if kind.is_box() {
        Annotation::new_box(kind, position)?
    } else {
        Annotation::new_line(
            kind,
            [position[0], position[0] + position[2]],
            [position[1], position[1] + position[3]],
        )?
    };
    if let Some(c) = p.get("Color").filter(|v| v.text().is_none()) {
        ann.color = color(c)?;
    }
    if let Some(edge) = p.get("EdgeColor") {
        if edge.text().is_some_and(|t| t.eq_ignore_ascii_case("none")) {
            ann.edge_none = true;
        } else {
            ann.color = color(edge)?;
            ann.edge_none = false;
        }
    }
    ann.line_width = scalar(p, "LineWidth", ann.line_width)?;
    ann.pattern = parse_line_pattern_str(string(p, "LineStyle", "-")?)?;
    ann.head_style = match string(p, "HeadStyle", "vback2")?
        .to_ascii_lowercase()
        .as_str()
    {
        "vback2" => AnnotationHeadStyle::VBack2,
        "plain" => AnnotationHeadStyle::Plain,
        "none" => AnnotationHeadStyle::None,
        _ => return Err("Unsupported FIG annotation HeadStyle".into()),
    };
    ann.head_length = scalar(p, "HeadLength", ann.head_length)?;
    ann.head_width = scalar(p, "HeadWidth", ann.head_width)?;
    ann.text = string(p, "String", "")?.to_owned();
    ann.font_size = scalar(p, "FontSize", ann.font_size)?;
    if let Some(face) = p.get("FaceColor") {
        ann.face_color = if face.text().is_some_and(|t| t.eq_ignore_ascii_case("none")) {
            None
        } else {
            Some(color(face)?)
        };
    }
    ann.face_alpha = scalar(p, "FaceAlpha", ann.face_alpha)?;
    ann.visible = match string(p, "Visible", "on")? {
        "on" => true,
        "off" => false,
        _ => return Err("FIG annotation Visible must be on or off".into()),
    };
    ann.validate()?;
    Ok(ann)
}

fn encode_fig_constant_line(
    cl: &crate::plot::ConstantLineObject,
    handle: f64,
) -> Vec<(&'static str, mat::Encoded<'static>)> {
    let mut props = vec![
        ("InterceptAxis", mat::text(cl.axis.as_str())),
        ("Value", num(&[cl.value])),
        ("LineStyle", mat::text(line_pattern_str(cl.pattern))),
        ("LineWidth", num(&[cl.line_width])),
        ("Alpha", num(&[cl.alpha])),
        ("Label", mat::text(&cl.label)),
        (
            "LabelHorizontalAlignment",
            mat::text(cl.label_h_align.as_str()),
        ),
        (
            "LabelVerticalAlignment",
            mat::text(cl.label_v_align.as_str()),
        ),
        ("DisplayName", mat::text(&cl.display_name)),
        ("Visible", mat::text(if cl.visible { "on" } else { "off" })),
    ];
    if let Some(rgb) = cl.color {
        props.push(("Color", num(&rgb)));
    }
    node(
        "matlab.graphics.chart.decoration.ConstantLine",
        handle,
        props,
        vec![],
        &[],
    )
}

fn decode_fig_constant_line(cl_node: &Fields<'_>) -> Result<crate::plot::ConstantLineObject> {
    use crate::plot::{ConstantLineHAlign, ConstantLineObject, ConstantLineVAlign, InterceptAxis};
    let p = object(field(cl_node, "properties")?)?;
    let axis = match string(p, "InterceptAxis", "x")?
        .to_ascii_lowercase()
        .as_str()
    {
        "x" => InterceptAxis::X,
        "y" => InterceptAxis::Y,
        _ => return Err("FIG ConstantLine InterceptAxis must be x or y".into()),
    };
    let value = scalar(p, "Value", f64::NAN)?;
    let mut cl = ConstantLineObject::new(axis, value);
    cl.color = optional_axes_rgb(p, "Color")?;
    cl.pattern = parse_line_pattern_str(string(p, "LineStyle", "-")?)?;
    cl.line_width = scalar(p, "LineWidth", cl.line_width)?;
    cl.alpha = scalar(p, "Alpha", cl.alpha)?;
    cl.label = string(p, "Label", "")?.to_owned();
    cl.label_h_align = ConstantLineHAlign::parse(string(p, "LabelHorizontalAlignment", "right")?)?;
    cl.label_v_align = ConstantLineVAlign::parse(string(p, "LabelVerticalAlignment", "top")?)?;
    cl.display_name = string(p, "DisplayName", "")?.to_owned();
    cl.visible = match string(p, "Visible", "on")? {
        "on" => true,
        "off" => false,
        _ => return Err("FIG ConstantLine Visible must be on or off".into()),
    };
    cl.validate()?;
    Ok(cl)
}

fn encode_fig_rectangle(
    rect: &crate::plot::RectangleObject,
    handle: f64,
) -> Vec<(&'static str, mat::Encoded<'static>)> {
    let props = vec![
        ("Position", num(&rect.position)),
        ("Curvature", num(&rect.curvature)),
        (
            "FaceColor",
            rect.face_color
                .map_or_else(|| mat::text("none"), |rgb| num(&rgb)),
        ),
        (
            "EdgeColor",
            rect.edge_color
                .map_or_else(|| mat::text("none"), |rgb| num(&rgb)),
        ),
        ("LineWidth", num(&[rect.line_width])),
        ("LineStyle", mat::text(line_pattern_str(rect.pattern))),
        (
            "Visible",
            mat::text(if rect.visible { "on" } else { "off" }),
        ),
    ];
    node("rectangle", handle, props, vec![], &[])
}

fn decode_fig_rectangle(rect_node: &Fields<'_>) -> Result<crate::plot::RectangleObject> {
    let p = object(field(rect_node, "properties")?)?;
    let mut rect = crate::plot::RectangleObject::new();
    let pos = field(p, "Position")?;
    if pos.len() != 4 {
        return Err("FIG rectangle Position must contain four numbers".into());
    }
    rect.position = [
        pos.number(0).ok_or("Invalid FIG rectangle Position")?,
        pos.number(1).ok_or("Invalid FIG rectangle Position")?,
        pos.number(2).ok_or("Invalid FIG rectangle Position")?,
        pos.number(3).ok_or("Invalid FIG rectangle Position")?,
    ];
    if let Some(curv) = p.get("Curvature") {
        let cx = curv.number(0).ok_or("Invalid FIG rectangle Curvature")?;
        let cy = curv.number(1).unwrap_or(cx);
        rect.curvature = [cx, cy];
    }
    if let Some(face) = p.get("FaceColor") {
        rect.face_color = if face.text().is_some_and(|t| t.eq_ignore_ascii_case("none")) {
            None
        } else {
            Some(color(face)?.map(|c| f64::from(c) / 255.))
        };
    }
    if let Some(edge) = p.get("EdgeColor") {
        rect.edge_color = if edge.text().is_some_and(|t| t.eq_ignore_ascii_case("none")) {
            None
        } else {
            Some(color(edge)?.map(|c| f64::from(c) / 255.))
        };
    }
    rect.line_width = scalar(p, "LineWidth", rect.line_width)?;
    rect.pattern = parse_line_pattern_str(string(p, "LineStyle", "-")?)?;
    rect.visible = match string(p, "Visible", "on")? {
        "on" => true,
        "off" => false,
        _ => return Err("FIG rectangle Visible must be on or off".into()),
    };
    rect.validate()?;
    Ok(rect)
}

fn encode_fig_label_properties<'a>(
    label: &'a str,
    source: Option<&'a crate::plot::text_markup::TextSource>,
    interpreter: crate::plot::text_markup::Interpreter,
) -> Result<Vec<(&'static str, mat::Encoded<'a>)>> {
    use crate::plot::text_markup::TextSource;
    let mut props = Vec::new();
    match source {
        Some(TextSource::Cell(lines)) if !lines.is_empty() => {
            let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
            props.push((
                "String",
                mat::text_cells(&refs, "FIG label line exceeds 4096 UTF-8 bytes")?,
            ));
        }
        _ => props.push(("String", mat::text(label))),
    }
    props.push(("Interpreter", mat::text(interpreter.as_str())));
    match source {
        Some(TextSource::Strings { rows, cols, values }) => {
            let present: Vec<f64> = values
                .iter()
                .map(|v| if v.is_some() { 1. } else { 0. })
                .collect();
            let strs: Vec<&str> = values.iter().map(|v| v.as_deref().unwrap_or("")).collect();
            props.push((
                "UserData",
                mat::structure(
                    "",
                    vec![(
                        "MatRustTextSource",
                        mat::structure(
                            "",
                            vec![
                                ("Kind", mat::text("strings")),
                                ("Rows", num(&[*rows as f64])),
                                ("Cols", num(&[*cols as f64])),
                                ("Present", mat::numbers(present.into_iter())),
                                (
                                    "Values",
                                    mat::text_cells(
                                        &strs,
                                        "FIG label string exceeds 4096 UTF-8 bytes",
                                    )?,
                                ),
                            ],
                        ),
                    )],
                ),
            ));
        }
        Some(TextSource::Character { rows, cols, units }) => {
            props.push((
                "UserData",
                mat::structure(
                    "",
                    vec![(
                        "MatRustTextSource",
                        mat::structure(
                            "",
                            vec![
                                ("Kind", mat::text("character")),
                                ("Rows", num(&[*rows as f64])),
                                ("Cols", num(&[*cols as f64])),
                                ("Units", mat::numbers(units.iter().map(|&u| f64::from(u)))),
                            ],
                        ),
                    )],
                ),
            ));
        }
        _ => {}
    }
    Ok(props)
}

fn decode_fig_label_string(
    p: &Fields<'_>,
) -> Result<(String, Option<crate::plot::text_markup::TextSource>)> {
    use crate::plot::text_markup::TextSource;
    let mut source = None;
    let text = match p.get("String") {
        None => String::new(),
        Some(Value::Text(s)) => s.clone(),
        Some(Value::Cell(items)) => {
            let lines: Vec<String> = items
                .iter()
                .map(|v| {
                    v.text()
                        .map(str::to_owned)
                        .ok_or_else(|| "Unsupported FIG label cell entry".to_owned())
                })
                .collect::<Result<Vec<_>>>()?;
            let joined = lines.join("\n");
            source = Some(TextSource::Cell(lines));
            joined
        }
        _ => return Err("Unsupported FIG label String".into()),
    };
    if let Some(ud) = p
        .get("UserData")
        .and_then(Value::structs)
        .and_then(|s| s.first())
        && let Some(ts) = ud
            .get("MatRustTextSource")
            .and_then(Value::structs)
            .and_then(|s| s.first())
    {
        let kind = string(ts, "Kind", "")?;
        let rows = scalar(ts, "Rows", 0.)? as usize;
        let cols = scalar(ts, "Cols", 0.)? as usize;
        match kind {
            "strings" => {
                let present = field(ts, "Present")?;
                let values_cell = match field(ts, "Values")? {
                    Value::Cell(c) => c,
                    _ => return Err("Invalid FIG MatRustTextSource Values".into()),
                };
                if present.len() != values_cell.len() {
                    return Err("Invalid FIG MatRustTextSource length".into());
                }
                let values = (0..values_cell.len())
                    .map(|i| {
                        if present.number(i).unwrap_or(0.) == 0. {
                            Ok(None)
                        } else {
                            values_cell[i]
                                .text()
                                .map(|s| Some(s.to_owned()))
                                .ok_or_else(|| "Invalid FIG MatRustTextSource entry".to_owned())
                        }
                    })
                    .collect::<Result<Vec<_>>>()?;
                source = Some(TextSource::Strings { rows, cols, values });
            }
            "character" => {
                let units_val = field(ts, "Units")?;
                let units = (0..units_val.len())
                    .map(|i| {
                        units_val
                            .number(i)
                            .map(|v| v as u16)
                            .ok_or_else(|| "Invalid FIG MatRustTextSource unit".to_owned())
                    })
                    .collect::<Result<Vec<_>>>()?;
                source = Some(TextSource::Character { rows, cols, units });
            }
            _ => return Err("Unsupported FIG MatRustTextSource kind".into()),
        }
    }
    Ok((text, source))
}

fn encode_fig_axes_extras<'a>(
    axes: &'a crate::plot::Axes,
    props: &mut Vec<(&'static str, mat::Encoded<'a>)>,
) -> Result<()> {
    if axes.extras.layer.is_top() {
        props.push(("Layer", mat::text(axes.extras.layer.as_str())));
    }
    if let Some(cmap) = &axes.extras.colormap {
        let rows = cmap.len();
        let mut flat = Vec::with_capacity(rows * 3);
        for c in 0..3 {
            for row in cmap.iter() {
                flat.push(row[c]);
            }
        }
        props.push(("Colormap", mat::matrix(rows, 3, flat)));
    }
    if let Some(clim) = axes.extras.clim {
        props.push(("CLim", num(&clim)));
        props.push(("CLimMode", mat::text("manual")));
    }
    for (axis, tick_key, label_key, rot_key) in [
        (0, "XTick", "XTickLabel", "XTickLabelRotation"),
        (1, "YTick", "YTickLabel", "YTickLabelRotation"),
        (2, "ZTick", "ZTickLabel", "ZTickLabelRotation"),
    ] {
        let ruler = &axes.extras.rulers[axis];
        if let Some(ticks) = &ruler.ticks {
            props.push((tick_key, mat::numbers(ticks.iter().copied())));
        }
        if let Some(labels) = &ruler.tick_labels {
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            props.push((
                label_key,
                mat::text_cells(&refs, "FIG tick label exceeds 4096 UTF-8 bytes")?,
            ));
        }
        if ruler.tick_label_rotation != 0. {
            props.push((rot_key, num(&[ruler.tick_label_rotation])));
        }
    }
    if axes.extras.tick_label_interpreter != crate::plot::text_markup::Interpreter::Tex {
        props.push((
            "TickLabelInterpreter",
            mat::text(axes.extras.tick_label_interpreter.as_str()),
        ));
    }
    let needs_ruler_meta = axes
        .extras
        .rulers
        .iter()
        .any(|r| !r.kind.is_numeric() || r.tick_label_format.is_some() || r.exponent != 0);
    if needs_ruler_meta {
        let mut ruler_structs = Vec::with_capacity(3);
        for (key, ruler) in [
            ("X", &axes.extras.rulers[0]),
            ("Y", &axes.extras.rulers[1]),
            ("Z", &axes.extras.rulers[2]),
        ] {
            let mut fields = vec![
                ("Kind", mat::text(ruler.kind.kind_word())),
                ("Exponent", num(&[f64::from(ruler.exponent)])),
            ];
            if let Some(fmt) = &ruler.tick_label_format {
                fields.push(("TickLabelFormat", mat::text(fmt)));
            }
            match &ruler.kind {
                crate::plot::RulerKind::Numeric => {}
                crate::plot::RulerKind::Datetime { utc, format } => {
                    fields.push(("Utc", num(&[if *utc { 1. } else { 0. }])));
                    fields.push(("Format", mat::text(format)));
                }
                crate::plot::RulerKind::Duration { format, unit_ms } => {
                    fields.push(("Format", mat::text(format)));
                    fields.push(("UnitMs", num(&[*unit_ms])));
                }
                crate::plot::RulerKind::Categorical {
                    categories,
                    ordinal,
                } => {
                    let refs: Vec<&str> = categories.iter().map(String::as_str).collect();
                    fields.push((
                        "Categories",
                        mat::text_cells(&refs, "FIG category name exceeds 4096 UTF-8 bytes")?,
                    ));
                    fields.push(("Ordinal", num(&[if *ordinal { 1. } else { 0. }])));
                }
            }
            ruler_structs.push((key, mat::structure("", fields)));
        }
        props.push((
            "UserData",
            mat::structure(
                "",
                vec![("MatRustRulers", mat::structure("", ruler_structs))],
            ),
        ));
    }
    Ok(())
}

fn decode_fig_axes_extras(props: &Fields<'_>, axes: &mut crate::plot::Axes) -> Result<()> {
    axes.extras.layer = crate::plot::AxesLayer::parse(string(props, "Layer", "bottom")?)?;
    if let Some(cmap_val) = props.get("Colormap") {
        let len = cmap_val.len();
        if len == 0 || !len.is_multiple_of(3) {
            return Err("FIG Colormap must be an Nx3 matrix with 1..4096 rows".into());
        }
        let rows = len / 3;
        if rows > crate::plot::MAX_COLORMAP_ROWS {
            return Err("FIG Colormap must be an Nx3 matrix with 1..4096 rows".into());
        }
        let mut cmap = Vec::with_capacity(rows);
        for r in 0..rows {
            let rgb = [
                cmap_val.number(r).ok_or("Invalid FIG Colormap")?,
                cmap_val.number(rows + r).ok_or("Invalid FIG Colormap")?,
                cmap_val
                    .number(2 * rows + r)
                    .ok_or("Invalid FIG Colormap")?,
            ];
            if rgb
                .iter()
                .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
            {
                return Err("FIG Colormap values must be finite in 0..1".into());
            }
            cmap.push(rgb);
        }
        axes.extras.colormap = Some(std::sync::Arc::new(cmap));
    }
    if let Some(clim) = limits(props, "CLim")? {
        if clim[0] >= clim[1] {
            return Err("FIG CLim must be strictly increasing".into());
        }
        axes.extras.clim = Some(clim);
    }
    for (axis, tick_key, label_key, rot_key) in [
        (0, "XTick", "XTickLabel", "XTickLabelRotation"),
        (1, "YTick", "YTickLabel", "YTickLabelRotation"),
        (2, "ZTick", "ZTickLabel", "ZTickLabelRotation"),
    ] {
        if let Some(ticks_val) = props.get(tick_key) {
            let mut ticks = Vec::with_capacity(ticks_val.len());
            for i in 0..ticks_val.len() {
                ticks.push(ticks_val.number(i).ok_or("Invalid FIG ticks")?);
            }
            axes.extras.rulers[axis].ticks = Some(ticks);
        }
        if let Some(labels_val) = props.get(label_key) {
            let labels = match labels_val {
                Value::Cell(items) => items
                    .iter()
                    .map(|v| {
                        v.text()
                            .map(str::to_owned)
                            .ok_or_else(|| "Invalid FIG tick label".to_owned())
                    })
                    .collect::<Result<Vec<_>>>()?,
                Value::Text(s) => vec![s.clone()],
                _ => return Err("Unsupported FIG tick labels".into()),
            };
            axes.extras.rulers[axis].tick_labels = Some(labels);
        }
        if props.contains_key(rot_key) {
            axes.extras.rulers[axis].tick_label_rotation = scalar(props, rot_key, 0.)?;
        }
    }
    axes.extras.tick_label_interpreter = crate::plot::text_markup::Interpreter::parse(string(
        props,
        "TickLabelInterpreter",
        "tex",
    )?)?;
    if let Some(ud) = props
        .get("UserData")
        .and_then(Value::structs)
        .and_then(|s| s.first())
        && let Some(rulers) = ud
            .get("MatRustRulers")
            .and_then(Value::structs)
            .and_then(|s| s.first())
    {
        for (axis, key) in [(0, "X"), (1, "Y"), (2, "Z")] {
            let Some(r) = rulers
                .get(key)
                .and_then(Value::structs)
                .and_then(|s| s.first())
            else {
                continue;
            };
            axes.extras.rulers[axis].exponent = scalar(r, "Exponent", 0.)? as i32;
            if r.contains_key("TickLabelFormat") {
                axes.extras.rulers[axis].tick_label_format =
                    Some(string(r, "TickLabelFormat", "")?.to_owned());
            }
            axes.extras.rulers[axis].kind = match string(r, "Kind", "numeric")? {
                "numeric" => crate::plot::RulerKind::Numeric,
                "datetime" => crate::plot::RulerKind::Datetime {
                    utc: scalar(r, "Utc", 0.)? != 0.,
                    format: string(r, "Format", "")?.to_owned(),
                },
                "duration" => crate::plot::RulerKind::Duration {
                    format: string(r, "Format", "")?.to_owned(),
                    unit_ms: scalar(r, "UnitMs", 1_000.)?,
                },
                "categorical" => {
                    let categories = match r.get("Categories") {
                        Some(Value::Cell(items)) => items
                            .iter()
                            .map(|v| {
                                v.text()
                                    .map(str::to_owned)
                                    .ok_or_else(|| "Invalid FIG category".to_owned())
                            })
                            .collect::<Result<Vec<_>>>()?,
                        _ => Vec::new(),
                    };
                    crate::plot::RulerKind::Categorical {
                        categories,
                        ordinal: scalar(r, "Ordinal", 0.)? != 0.,
                    }
                }
                other => return Err(format!("Unsupported FIG ruler kind: {other}")),
            };
        }
    }
    for ruler in &axes.extras.rulers {
        ruler.validate()?;
    }
    Ok(())
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    type EncodedNode = Vec<(&'static str, mat::Encoded<'static>)>;
    fn axes(
        handle: f64,
        position: [f64; 4],
        kind: &str,
        samples: usize,
        count: usize,
        label: &str,
    ) -> EncodedNode {
        let children = (0..count)
            .map(|i| {
                node(
                    kind,
                    100. + handle * 100. + i as f64,
                    vec![
                        ("XData", mat::numbers((0..samples).map(|i| i as f64 + 1.))),
                        (
                            "YData",
                            mat::numbers((0..samples).map(|i| i as f64 + handle)),
                        ),
                        ("DisplayName", mat::text(label)),
                    ],
                    vec![],
                    &[],
                )
            })
            .collect();
        node(
            "axes",
            handle,
            vec![
                ("Units", mat::text("normalized")),
                ("Position", num(&position)),
            ],
            children,
            &[0.; 4],
        )
    }
    fn legend(handle: f64, peer: f64, label: &str, visible: bool) -> EncodedNode {
        node(
            "scribe.legend",
            handle,
            vec![
                ("Axes", num(&[peer])),
                ("String", mat::text(label)),
                ("Visible", mat::text(if visible { "on" } else { "off" })),
            ],
            vec![],
            &[],
        )
    }
    fn file(children: Vec<EncodedNode>, current: f64) -> Vec<u8> {
        mat::file(mat::structure(
            "hgS_070000",
            node(
                "figure",
                1.,
                vec![("CurrentAxes", num(&[current]))],
                children,
                &[],
            ),
        ))
        .unwrap()
    }
    const TOP: [f64; 4] = [0.1, 0.6, 0.8, 0.3];
    const BOTTOM: [f64; 4] = [0.1, 0.1, 0.8, 0.3];
    #[test]
    fn root_figure_handle_is_a_finite_scalar_without_integer_or_positive_restriction() {
        fn encoded(handle: Option<mat::Encoded<'static>>) -> Vec<u8> {
            let mut root = node(
                "figure",
                1.,
                vec![("CurrentAxes", num(&[2.]))],
                vec![axes(2., TOP, "line", 2, 1, "One")],
                &[],
            );
            root.retain(|(name, _)| *name != "handle");
            if let Some(handle) = handle {
                root.push(("handle", handle));
            }
            mat::file(mat::structure("hgS_070000", root)).unwrap()
        }
        for handle in [0., -0., 1., -3.5, 150.00830078125] {
            let figure = decode_matlab_figure(&encoded(Some(num(&[handle])))).unwrap();
            assert_eq!(figure.series.len(), 1);
            assert_eq!(figure.series[0].points.len(), 2);
        }
        for handle in [
            None,
            Some(num(&[])),
            Some(num(&[1., 2.])),
            Some(mat::text("not a handle")),
            Some(num(&[f64::NAN])),
            Some(num(&[f64::INFINITY])),
            Some(num(&[f64::NEG_INFINITY])),
        ] {
            assert!(decode_matlab_figure(&encoded(handle)).is_err());
        }
    }

    #[test]
    fn extended_fig_locations_preserve_solved_axes_and_validate_legend_layout() {
        // Synthetic hierarchy mutations exercise public-property validation;
        // they are not claimed as newly generated MATLAB-origin fixtures.
        let inner = [0.38, 0.35, 0.24, 0.3];
        let make = |location, orientation, columns, modern, legacy| {
            let line = node(
                "graph2d.lineseries",
                3.,
                vec![
                    ("XData", num(&[0., 1.])),
                    ("YData", num(&[0., 1.])),
                    ("DisplayName", mat::text("Signal")),
                ],
                vec![],
                &[],
            );
            let axes = node(
                "axes",
                2.,
                vec![
                    ("Units", mat::text("normalized")),
                    ("Position", num(&inner)),
                    ("OuterPosition", num(&[0., 0., 1., 1.])),
                    ("PositionConstraint", mat::text(modern)),
                    ("ActivePositionProperty", mat::text(legacy)),
                ],
                vec![line],
                &[0.; 4],
            );
            let legend = node(
                "scribe.legend",
                20.,
                vec![
                    ("Axes", num(&[2.])),
                    ("String", mat::text("Signal")),
                    ("Location", mat::text(location)),
                    ("Orientation", mat::text(orientation)),
                    ("NumColumns", num(&[columns])),
                ],
                vec![],
                &[],
            );
            decode_matlab_figure(&file(vec![axes, legend], 2.))
        };
        for location in crate::plot::LegendLocation::ALL {
            let figure = make(
                location.name(),
                "vertical",
                1.,
                "outerposition",
                "outerposition",
            )
            .unwrap();
            assert_eq!(figure.legend_location, location);
            let placement = figure.placement.unwrap();
            assert!(placement.imported_solved);
            assert_eq!(placement.requested_inner, inner);
            assert_eq!(
                placement.constraint,
                crate::plot::PositionConstraint::OuterPosition
            );
            for (actual, expected) in figure.resolved_axes_positions().unwrap()[0]
                .1
                .into_iter()
                .zip(inner)
            {
                assert!((actual - expected).abs() < 1e-12);
            }
        }
        for (name, orientation, columns) in [
            ("horizontal", crate::plot::LegendOrientation::Horizontal, 1),
            ("vertical", crate::plot::LegendOrientation::Vertical, 2),
        ] {
            let figure = make(
                "eastoutside",
                name,
                columns as f64,
                "outerposition",
                "outerposition",
            )
            .unwrap();
            assert_eq!(figure.extras.legend_orientation, orientation);
            assert_eq!(figure.extras.legend_num_columns, Some(columns));
            assert!(figure.placement.unwrap().imported_solved);
            for (actual, expected) in figure.resolved_axes_positions().unwrap()[0]
                .1
                .into_iter()
                .zip(inner)
            {
                assert!((actual - expected).abs() < 1e-12);
            }
        }
        assert!(
            make(
                "eastoutside",
                "diagonal",
                1.,
                "outerposition",
                "outerposition"
            )
            .is_err()
        );
        assert!(
            make(
                "eastoutside",
                "vertical",
                0.,
                "outerposition",
                "outerposition"
            )
            .is_err()
        );
        assert!(make("eastoutside", "vertical", 1., "outerposition", "position").is_err());
        assert!(make("layout", "vertical", 1., "outerposition", "outerposition").is_err());
    }
    #[test]
    fn legend_location_fig_import_preserves_supported_and_rejects_unknown_positions() {
        for value in [
            "north",
            "south",
            "east",
            "west",
            "northeast",
            "northwest",
            "southeast",
            "southwest",
            "best",
            "southoutside",
            "none",
        ] {
            let legend = node(
                "scribe.legend",
                20.,
                vec![
                    ("Axes", num(&[2.])),
                    ("String", mat::text("Signal")),
                    ("Location", mat::text(value)),
                ],
                vec![],
                &[],
            );
            let result = decode_matlab_figure(&file(
                vec![axes(2., TOP, "line", 2, 1, "Signal"), legend],
                2.,
            ));
            match crate::plot::LegendLocation::parse(value) {
                Ok(location) => assert_eq!(result.unwrap().legend_location, location),
                Err(_) => assert!(result.unwrap_err().contains("Location")),
            }
        }
    }
    #[test]
    fn fig_legend_peer_single_axes_accepts_exact_fractional_and_zero_handles() {
        for peer in [0., 2., 0.0001220703125] {
            let figure = decode_matlab_figure(&file(
                vec![
                    axes(peer, TOP, "line", 2, 1, "Signal"),
                    legend(20., peer, "Signal", true),
                ],
                peer,
            ))
            .unwrap();
            assert!(figure.legend);
            assert_eq!(figure.series[0].label, "Signal");
            assert_eq!(figure.series[0].points.len(), 2);
        }
    }

    #[test]
    fn fig_legend_peer_rejects_explicit_invalid_values_even_with_single_axes() {
        for peer in [
            num(&[99.]),
            num(&[1.]), // The figure handle is not an axes handle.
            num(&[f64::NAN]),
            num(&[f64::INFINITY]),
            num(&[f64::NEG_INFINITY]),
            num(&[]),
            num(&[2., 2.]),
            mat::text("2"),
            mat::structure("", vec![("peer", num(&[2.]))]),
        ] {
            let bad = node(
                "scribe.legend",
                20.,
                vec![("Axes", peer), ("String", mat::text("Signal"))],
                vec![],
                &[],
            );
            let error =
                decode_matlab_figure(&file(vec![axes(2., TOP, "line", 2, 1, "Signal"), bad], 2.))
                    .unwrap_err();
            assert!(error.contains("Axes"), "{error}");
        }
    }

    #[test]
    fn fig_legend_peer_omission_is_unambiguous_only_with_single_axes() {
        let missing = || {
            node(
                "scribe.legend",
                20.,
                vec![("String", mat::text("Signal"))],
                vec![],
                &[],
            )
        };
        let figure = decode_matlab_figure(&file(
            vec![axes(2., TOP, "line", 2, 1, "Signal"), missing()],
            2.,
        ))
        .unwrap();
        assert!(figure.legend);
        assert_eq!(figure.series[0].label, "Signal");
        let error = decode_matlab_figure(&file(
            vec![
                axes(2., TOP, "line", 2, 1, "Signal"),
                axes(3., BOTTOM, "line", 2, 1, "Other"),
                missing(),
            ],
            2.,
        ))
        .unwrap_err();
        assert!(error.contains("Axes peer is required"), "{error}");
    }

    #[test]
    fn fig_legend_peer_authentic_matlab_fixtures_keep_known_labels() {
        for (name, labels) in [
            ("matlab-lines.fig", ["First", "Second"]),
            ("matlab-lines-v6.fig", ["First", "Second"]),
            ("matlab-log-scatter.fig", ["Line", "Samples"]),
            ("matlab-bars.fig", ["A", "B"]),
        ] {
            let path = Path::new("tests/fixtures/fig").join(name);
            let figure = load_matlab_figure(&path).unwrap();
            assert!(figure.legend, "{name}");
            assert_eq!(figure.series.len(), labels.len(), "{name}");
            for (series, expected) in figure.series.iter().zip(labels) {
                assert_eq!(series.label, expected, "{name}");
            }
        }
    }

    #[test]
    fn multiple_axes_legends_are_bound_by_peer_handles() {
        let figure = decode_matlab_figure(&file(
            vec![
                axes(2., TOP, "line", 2, 1, "One"),
                axes(3., BOTTOM, "line", 2, 1, "Two"),
                legend(30., 3., "Two", false),
                legend(20., 2., "One", true),
            ],
            3.,
        ))
        .unwrap();
        assert!(!figure.legend);
        assert_eq!(figure.series[0].label, "Two");
        let other = &figure.layout.as_ref().unwrap().others[0].axes;
        assert!(other.legend);
        assert_eq!(other.series[0].label, "One");
        for peer in [2., 99.] {
            let bytes = file(
                vec![
                    axes(2., TOP, "line", 2, 1, "One"),
                    axes(3., BOTTOM, "line", 2, 1, "Two"),
                    legend(20., 2., "One", true),
                    legend(30., peer, "One", true),
                ],
                3.,
            );
            assert!(decode_matlab_figure(&bytes).is_err());
        }
    }
    #[test]
    fn multiple_axes_admission_rejects_bad_geometry_handles_and_total_budgets() {
        for (handle, position, current) in [(2., BOTTOM, 2.), (3., TOP, 2.), (3., BOTTOM, 9.)] {
            let bytes = file(
                vec![
                    axes(2., TOP, "line", 2, 1, "One"),
                    axes(handle, position, "line", 2, 1, "Two"),
                ],
                current,
            );
            assert!(decode_matlab_figure(&bytes).is_err());
        }
        for (kind, n, m, count_a, count_b) in [
            ("line", 600000, 500000, 1, 1),
            ("line", 1, 1, 32, 33),
            ("specgraph.barseries", 6000, 5000, 1, 1),
        ] {
            let bytes = file(
                vec![
                    axes(2., TOP, kind, n, count_a, "One"),
                    axes(3., BOTTOM, kind, m, count_b, "Two"),
                ],
                2.,
            );
            let error = decode_matlab_figure(&bytes).unwrap_err();
            assert!(error.contains("budget"), "{error}");
        }
    }
}

#[cfg(test)]
mod width_serialization_tests {
    use super::*;
    #[test]
    fn line_width_fig_writer_preserves_other_legacy_serialized_fields() {
        let mut engine = crate::Engine::default();
        engine.execute("semilogx([1 10 100],[1 2 4],'r--o');hold on;plot([1 10 100],[3 4 6],'b:');title('Rust figure');xlabel('Time');ylabel('Amplitude');axis([1 100 0 8]);grid on;").unwrap();
        // This exact historical width-only comparison retains the legacy canvas policy.
        engine.figure.as_mut().unwrap().appearance = None;
        assert!(engine.figure.as_ref().unwrap().zgrid);
        let bytes = encode_matlab_figure(engine.figure.as_ref().unwrap()).unwrap();
        let legacy = include_bytes!("../../tests/fixtures/fig/legacy-writer-lines.fig");
        assert_eq!(&bytes[..128], &legacy[..128]);
        let mut actual = mat::hierarchy(&bytes[128..], mat::Endian::Little).unwrap();
        let expected = mat::hierarchy(&legacy[128..], mat::Endian::Little).unwrap();
        let Value::Struct(roots) = &mut actual else {
            panic!("expected FIG root structure");
        };
        let Value::Struct(properties) = roots[0].get_mut("properties").unwrap() else {
            panic!("expected FIG figure properties");
        };
        let properties = &mut properties[0];
        for (key, value) in [
            ("Units", "pixels"),
            ("Resize", "on"),
            ("NumberTitle", "on"),
            ("WindowState", "normal"),
        ] {
            assert_eq!(properties.remove(key).unwrap().text(), Some(value));
        }
        let position = properties.remove("Position").unwrap();
        assert_eq!(position.len(), 4);
        assert_eq!(
            std::array::from_fn(|i| position.number(i).unwrap()),
            [100., 100., 960., 560.]
        );
        assert_eq!(properties["Visible"].text(), Some("on"));
        properties.insert("Visible".into(), Value::Text("off".into()));
        fn strip_widths(value: &mut Value<'_>) -> usize {
            match value {
                Value::Struct(items) => items
                    .iter_mut()
                    .map(|fields| {
                        if let Some(data) = fields.remove("UserData") {
                            let metadata = object(&data).unwrap();
                            assert_eq!(metadata.len(), 1);
                            let hint =
                                object(field(metadata, "MatRustImplicitWidth").unwrap()).unwrap();
                            assert_eq!(hint.len(), 2);
                            assert_eq!(scalar(hint, "Version", 0.).unwrap(), 1.);
                            assert_eq!(scalar(hint, "ExportedValue", 0.).unwrap(), 1.5);
                        }
                        let removed = if let Some(width) = fields.remove("LineWidth") {
                            assert_eq!(width.len(), 1);
                            assert_eq!(width.number(0), Some(1.5));
                            1
                        } else {
                            0
                        };
                        removed + fields.values_mut().map(strip_widths).sum::<usize>()
                    })
                    .sum(),
                Value::Cell(items) => items.iter_mut().map(strip_widths).sum(),
                _ => 0,
            }
        }
        assert_eq!(strip_widths(&mut actual), 2);
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
    }
}

#[cfg(test)]
mod line_style_none_fig_tests {
    use super::*;
    fn file(style: &str, marker: &str, userdata: Option<mat::Encoded<'static>>) -> Vec<u8> {
        let mut props = vec![
            ("XData", num(&[1., 2.])),
            ("YData", num(&[3., 4.])),
            ("LineStyle", mat::text(style)),
            ("Marker", mat::text(marker)),
        ];
        if let Some(data) = userdata {
            props.push(("UserData", data));
        }
        let line = node("graph2d.lineseries", 3., props, vec![], &[]);
        let axes = node("axes", 2., vec![], vec![line], &[0.; 4]);
        mat::file(mat::structure(
            "hgS_070000",
            node(
                "figure",
                1.,
                vec![("CurrentAxes", num(&[2.]))],
                vec![axes],
                &[],
            ),
        ))
        .unwrap()
    }
    fn hint(version: f64, kind: &str, pattern: &str) -> mat::Encoded<'static> {
        mat::structure(
            "",
            vec![(
                "MatRustGraphics",
                mat::structure(
                    "",
                    vec![
                        ("Version", num(&[version])),
                        ("Kind", mat::text(kind)),
                        ("Pattern", mat::text(pattern)),
                    ],
                ),
            )],
        )
    }
    #[test]
    fn line_style_none_unhinted_line_classes_stay_lines_and_unrelated_userdata_is_ignored() {
        // Synthetic MAT structures exercise parser policy, not a MATLAB reference.
        for marker in ["none", "o"] {
            for data in [
                None,
                Some(num(&[1., 2.])),
                Some(mat::text("unrelated")),
                Some(mat::structure(
                    "",
                    vec![("Owner", mat::text("someone else"))],
                )),
            ] {
                let f = decode_matlab_figure(&file("none", marker, data)).unwrap();
                assert_eq!(f.series[0].kind, PlotKind::Line);
                assert_eq!(f.series[0].pattern, LinePattern::None);
                assert_eq!(f.series[0].markers, marker != "none");
                assert_eq!(
                    f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
                    &[[1., 3.], [2., 4.]]
                );
            }
        }
        let genuine =
            load_matlab_figure(Path::new("tests/fixtures/fig/matlab-log-scatter.fig")).unwrap();
        assert_eq!(genuine.series[1].kind, PlotKind::Scatter);
        assert_eq!(
            genuine.series[1]
                .points
                .iter()
                .collect::<Vec<_>>()
                .as_slice(),
            &[[1., 2.], [10., 3.], [100., 5.]]
        );
    }
    #[test]
    fn line_style_none_scatter_hint_retains_kind_and_dormant_pattern_on_new_exports() {
        for pattern in [
            LinePattern::Solid,
            LinePattern::Dashed,
            LinePattern::Dotted,
            LinePattern::DashDot,
            LinePattern::None,
        ] {
            let mut e = crate::Engine::default();
            e.execute("scatter([1 2],[3 4],'Marker','x','SizeData',144);")
                .unwrap();
            let mut f = e.figure.unwrap();
            f.series[0].pattern = pattern;
            let restored = decode_matlab_figure(&encode_matlab_figure(&f).unwrap()).unwrap();
            assert_eq!(restored.series[0].kind, PlotKind::Scatter);
            assert_eq!(restored.series[0].pattern, pattern);
            assert_eq!(restored.svg(), f.svg());
        }
    }
    #[test]
    fn line_style_none_rejects_malformed_or_contradictory_recognized_scatter_hints() {
        for data in [
            hint(2., "scatter", "Solid"),
            hint(f64::NAN, "scatter", "Solid"),
            hint(1., "line", "Solid"),
            hint(1., "scatter", "Unknown"),
            mat::structure("", vec![("MatRustGraphics", mat::text("bad"))]),
            mat::structures(
                "",
                vec![
                    vec![("MatRustGraphics", num(&[1.]))],
                    vec![("MatRustGraphics", num(&[1.]))],
                ],
            ),
        ] {
            assert!(decode_matlab_figure(&file("none", "o", Some(data))).is_err());
        }
        for (style, marker) in [("-", "o"), ("none", "none")] {
            let error =
                decode_matlab_figure(&file(style, marker, Some(hint(1., "scatter", "Solid"))))
                    .unwrap_err();
            assert!(error.contains("MatRustGraphics"), "{error}");
        }
    }
}

#[cfg(test)]
mod implicit_width_tests {
    use super::*;
    fn metadata(version: f64, exported: f64) -> mat::Encoded<'static> {
        mat::structure(
            "",
            vec![(
                "MatRustImplicitWidth",
                mat::structure(
                    "",
                    vec![
                        ("Version", num(&[version])),
                        ("ExportedValue", num(&[exported])),
                    ],
                ),
            )],
        )
    }
    fn file(kind: &str, width: Option<f64>, userdata: mat::Encoded<'static>) -> Vec<u8> {
        file_with_style(kind, width, userdata, "none")
    }
    fn file_with_style(
        kind: &str,
        width: Option<f64>,
        userdata: mat::Encoded<'static>,
        style: &str,
    ) -> Vec<u8> {
        let mut props = vec![
            ("XData", num(&[1., 2.])),
            ("YData", num(&[3., 4.])),
            ("LineStyle", mat::text(style)),
            ("Marker", mat::text("x")),
            ("UserData", userdata),
        ];
        if let Some(width) = width {
            props.push(("LineWidth", num(&[width])));
        }
        let line = node(kind, 3., props, vec![], &[]);
        let axes = node("axes", 2., vec![], vec![line], &[0.; 4]);
        mat::file(mat::structure(
            "hgS_070000",
            node(
                "figure",
                1.,
                vec![("CurrentAxes", num(&[2.]))],
                vec![axes],
                &[],
            ),
        ))
        .unwrap()
    }
    fn assert_public_width(value: &Value<'_>, expected: f64, implicit: bool) -> usize {
        match value {
            Value::Struct(items) => items
                .iter()
                .map(|fields| {
                    let here = if fields.contains_key("XData") {
                        assert_eq!(scalar(fields, "LineWidth", f64::NAN).unwrap(), expected);
                        let hint = fields
                            .get("UserData")
                            .and_then(Value::structs)
                            .and_then(|items| items[0].get("MatRustImplicitWidth"));
                        assert_eq!(hint.is_some(), implicit);
                        if let Some(hint) = hint {
                            let metadata = object(hint).unwrap();
                            assert_eq!(
                                scalar(metadata, "ExportedValue", f64::NAN).unwrap(),
                                expected
                            );
                        }
                        1
                    } else {
                        0
                    };
                    here + fields
                        .values()
                        .map(|v| assert_public_width(v, expected, implicit))
                        .sum::<usize>()
                })
                .sum(),
            Value::Cell(items) => items
                .iter()
                .map(|v| assert_public_width(v, expected, implicit))
                .sum(),
            _ => 0,
        }
    }
    #[test]
    fn implicit_and_explicit_width_roundtrips_preserve_rendering_and_semantics() {
        for kind in [PlotKind::Line, PlotKind::Scatter] {
            for pattern in [LinePattern::Solid, LinePattern::Dashed, LinePattern::None] {
                for markers in [false, true] {
                    for width in [None, Some(0.75), Some(1.5), Some(3.)] {
                        let mut e = crate::Engine::default();
                        e.execute("plot([1 2],[3 4],'Marker','x','MarkerSize',12);")
                            .unwrap();
                        let mut f = e.figure.unwrap();
                        let original = &mut f.series[0];
                        original.kind = kind;
                        original.pattern = pattern;
                        original.markers = markers || kind == PlotKind::Scatter;
                        original.line_width = width;
                        let bytes = encode_matlab_figure(&f).unwrap();
                        let public = width.unwrap_or(
                            if kind == PlotKind::Scatter || pattern == LinePattern::None {
                                0.75
                            } else {
                                1.5
                            },
                        );
                        assert_eq!(
                            assert_public_width(
                                &mat::hierarchy(&bytes[128..], mat::Endian::Little).unwrap(),
                                public,
                                width.is_none()
                            ),
                            1
                        );
                        let restored = decode_matlab_figure(&bytes).unwrap();
                        let actual = &restored.series[0];
                        assert_eq!(actual.line_width, width);
                        assert_eq!(actual.kind, kind);
                        assert_eq!(actual.pattern, pattern);
                        assert_eq!(actual.stroke_width(1.7), f.series[0].stroke_width(1.7));
                        assert_eq!(actual.stroke_width(1.), f.series[0].stroke_width(1.));
                        assert_eq!(restored.svg(), f.svg());
                        // A second exchange must not turn an implicit default explicit.
                        let twice = decode_matlab_figure(&encode_matlab_figure(&restored).unwrap())
                            .unwrap();
                        assert_eq!(twice.series[0].line_width, width);
                        assert_eq!(twice.svg(), f.svg());
                    }
                }
            }
        }
    }
    #[test]
    fn implicit_width_hint_allows_foreign_edits_and_missing_public_width() {
        for (public, expected) in [(Some(0.75), None), (Some(2.), Some(2.)), (None, Some(0.5))] {
            let f = decode_matlab_figure(&file("line", public, metadata(1., 0.75))).unwrap();
            assert_eq!(f.series[0].line_width, expected);
            assert_eq!(f.series[0].kind, PlotKind::Line);
        }
        let changed = decode_matlab_figure(&file_with_style(
            "line",
            Some(0.75),
            metadata(1., 0.75),
            "-",
        ))
        .unwrap();
        assert_eq!(changed.series[0].line_width, Some(0.75));
        let changed = decode_matlab_figure(&file("line", Some(1.5), metadata(1., 1.5))).unwrap();
        assert_eq!(changed.series[0].line_width, Some(1.5));
    }
    #[test]
    fn implicit_width_hint_rejects_malformed_nonfinite_and_contradictory_metadata() {
        for data in [
            metadata(2., 0.75),
            metadata(f64::NAN, 0.75),
            metadata(1., f64::INFINITY),
            metadata(1., f64::NAN),
            metadata(1., -0.75),
            metadata(1., 2.),
            mat::structure("", vec![("MatRustImplicitWidth", mat::text("bad"))]),
            mat::structure(
                "",
                vec![(
                    "MatRustImplicitWidth",
                    mat::structure(
                        "",
                        vec![
                            ("Version", num(&[1.])),
                            ("ExportedValue", num(&[0.75])),
                            ("Extra", num(&[1.])),
                        ],
                    ),
                )],
            ),
            mat::structures("", vec![vec![("MatRustImplicitWidth", num(&[1.]))], vec![]]),
        ] {
            assert!(decode_matlab_figure(&file("line", Some(0.75), data)).is_err());
        }
        for public in [f64::NAN, f64::INFINITY, -1.] {
            assert!(decode_matlab_figure(&file("line", Some(public), metadata(1., 0.75))).is_err());
        }
        assert!(decode_matlab_figure(&file("scatter", Some(2.), metadata(1., 0.75))).is_err());
        let error =
            decode_matlab_figure(&file("scatter", Some(0.75), metadata(1., 0.75))).unwrap_err();
        assert!(error.contains("MatRustImplicitWidth"), "{error}");
    }
}

#[cfg(test)]
mod figure_property_tests {
    use super::*;
    #[test]
    fn fig_public_root_name_color_are_genuine_properties() {
        let mut figure = Figure {
            name: "Independent name".into(),
            color: [18, 52, 86],
            appearance: None,
            ..Default::default()
        };
        figure
            .add(vec![Series::new(vec![[1., 2.]], PlotKind::Line)])
            .unwrap();
        let encoded = encode_matlab_figure(&figure).unwrap();
        let expanded = mat::expand(&encoded).unwrap();
        let hierarchy = expanded.hierarchy().unwrap();
        let root = object(&hierarchy).unwrap();
        let props = object(field(root, "properties").unwrap()).unwrap();
        assert_eq!(string(props, "Name", "").unwrap(), "Independent name");
        let value = field(props, "Color").unwrap();
        assert_eq!(value.len(), 3);
        for (i, c) in [18u8, 52, 86].into_iter().enumerate() {
            assert_eq!(value.number(i).unwrap(), f64::from(c) / 255.);
        }
    }
}

#[cfg(test)]
#[path = "fig_grid_writer_tests.rs"]
mod grid_writer_tests;
