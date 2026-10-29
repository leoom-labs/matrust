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
    for child in children(root)? {
        match string(child, "type", "")? {
            "axes" => axes_nodes.push(child),
            "scribe.legend" | "legend" => legend_nodes.push(child),
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
    if positioned {
        figure.layout = Some(crate::plot::Layout {
            active: positions[active],
            others: positions
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != active)
                .map(|(_, p)| crate::plot::Panel {
                    position: *p,
                    axes: Default::default(),
                })
                .collect(),
            tiled: None,
            title: String::new(),
        });
        figure.validate()?; // Reject invalid geometry before sample allocation.
        figure.layout.as_mut().unwrap().others.clear();
    }
    let mut points = 0;
    let mut series = 0;
    let mut bars = 0;
    for (i, axes) in axes_nodes.into_iter().enumerate() {
        let (visible, labels, location, legend_colors) = if let Some(legend) = legends[i] {
            let p = object(field(legend, "properties")?)?;
            if !string(p, "Orientation", "vertical")?.eq_ignore_ascii_case("vertical")
                || scalar(p, "NumColumns", 1.)? != 1.
            {
                return Err(
                    "FIG legend import currently requires vertical single-column layout".into(),
                );
            }
            if !string(p, "Box", "on")?.eq_ignore_ascii_case("on") {
                return Err("FIG legend Box off is not supported".into());
            }
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
            )
        } else {
            (false, None, crate::plot::LegendLocation::default(), None)
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
    if let Some(view) = props.get("View")
        && (view.number(0) != Some(0.) || view.number(1) != Some(90.))
    {
        return Err("Only 2-D FIG axes are supported".into());
    }
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
    // These explicit properties affect rendered grids but have no corresponding
    // native model yet. Do not silently discard a supplied style, even if hidden.
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
    for key in ["XMinorGrid", "YMinorGrid"] {
        if grid(key)? {
            return Err(format!(
                "Enabled FIG {key} is not supported; minor grids are not represented"
            ));
        }
    }
    let mut colors = crate::plot::AxesColors {
        grid: optional_grid_style(props)?,
        background: optional_axes_rgb(props, "Color")?,
        x: optional_axes_rgb(props, "XColor")?,
        y: optional_axes_rgb(props, "YColor")?,
        ..Default::default()
    };
    let xgrid = grid("XGrid")?;
    let ygrid = grid("YGrid")?;
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
            hold: string(props, "NextPlot", "replace")? == "add",
            xgrid,
            ygrid,
            legend,
            ..Default::default()
        },
        layout: None,
        textboxes: Vec::new(),
        name: String::new(),
        color: [255; 3],
        appearance: None,
        invert_hardcopy: None,
    };
    let children = children(axes)?;
    let special = field(axes, "special")?;
    if !matches!(special, Value::Number { .. }) {
        return Err("Invalid FIG label references".into());
    }
    let mut text_indices = Vec::new();
    for (i, label) in [&mut f.axes.title, &mut f.axes.xlabel, &mut f.axes.ylabel]
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
        *label = string(p, "String", "")?.to_owned();
        let rgb = optional_axes_rgb(p, "Color")?;
        match i {
            0 => colors.title = rgb,
            1 => colors.xlabel = rgb,
            _ => colors.ylabel = rgb,
        }
        text_indices.push(index);
    }
    if colors != crate::plot::AxesColors::default() {
        f.axes.colors = Some(colors);
    }
    for (index, node) in children.iter().enumerate() {
        let kind = string(node, "type", "")?;
        if kind == "text" && text_indices.contains(&index) || kind == "specgraph.baseline" {
            continue;
        }
        if kind == "text" && special.number(3) == Some(index as f64 + 1.) {
            let p = object(field(node, "properties")?)?;
            if string(p, "String", "")?.is_empty() {
                continue;
            }
            return Err("Nonempty FIG z-axis labels require 3-D support".into());
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
        if p.get("ZData")
            .is_some_and(|v| !matches!(v, Value::Number { count: 0, .. }))
        {
            return Err("3-D FIG series are not supported".into());
        }
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
        // This legacy importer does not reconstruct MATLAB BaseLine properties.
        // Preserve its established fill-only Bar appearance explicitly.
        series.bar_baseline_visible = false;
        if plot_kind != PlotKind::Bar {
            let width = scalar(p, "LineWidth", 0.5)?;
            crate::plot::validate_line_width(width)?;
            series.line_width = Some(width);
        }
        series.visible = visible;
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
        ("Interpreter", mat::text("none")),
        ("Orientation", mat::text("vertical")),
        (
            "ApplicationData",
            mat::structure(
                "",
                vec![("PlotChildren", mat::numbers(plan.members.iter().copied()))],
            ),
        ),
    ];
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
    let mut props = vec![
        ("Visible", mat::text("off")),
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
    let mut special = [0.; 4];
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
    let label_colors = [colors.title, colors.xlabel, colors.ylabel];
    for (i, label) in [&figure.title, &figure.xlabel, &figure.ylabel]
        .into_iter()
        .enumerate()
    {
        if !label.is_empty() || label_colors[i].is_some() {
            let mut text_properties = vec![
                ("String", mat::text(label)),
                ("Interpreter", mat::text("none")),
            ];
            if let Some(rgb) = label_colors[i] {
                text_properties.push(("Color", num(&rgb)));
            }
            children.push(node("text", *next_handle, text_properties, vec![], &[]));
            *next_handle += 1.;
            special[i] = children.len() as f64;
        }
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
        ("View", num(&[0., 90.])),
    ];
    for (property, rgb) in [
        ("Color", colors.background),
        ("XColor", colors.x),
        ("YColor", colors.y),
    ] {
        if let Some(rgb) = rgb {
            props.push((property, num(&rgb)));
        }
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
    fn extended_fig_locations_preserve_solved_axes_and_reject_unsupported_legend_layout() {
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
        assert!(
            make(
                "eastoutside",
                "horizontal",
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
                2.,
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
        let bytes = encode_matlab_figure(engine.figure.as_ref().unwrap()).unwrap();
        let legacy = include_bytes!("../../tests/fixtures/fig/legacy-writer-lines.fig");
        assert_eq!(&bytes[..128], &legacy[..128]);
        let mut actual = mat::hierarchy(&bytes[128..], mat::Endian::Little).unwrap();
        let expected = mat::hierarchy(&legacy[128..], mat::Endian::Little).unwrap();
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
