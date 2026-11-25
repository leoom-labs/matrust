//! Graphics objects through dot syntax (`h.Prop`, `h.Prop = v`), plus the
//! axes-owned objects `text`, `colorbar`, `colormap`, `clim`/`caxis`,
//! `yyaxis` and `drawnow`. Dot reads and writes route through the same
//! `get`/`set` code, so both forms always agree.
use super::{Engine, Execution, Located, Value, axes_rgb, handle, text};
use crate::ObjectIdentity;
use crate::plot::{
    Axes, Colorbar, ColorbarLocation, Figure, LabelStyle, Series, Subtitle, TextObject, YyAxis,
    colormap,
};
use crate::syntax::PathStep;
use crate::{Array, GraphicsArray, GraphicsHandle, GraphicsKind, LogicalMatrix, Matrix, Result};
use std::sync::Arc;

const MAP_FUNCTIONS: [&str; 9] = [
    "viridis", "parula", "gray", "hot", "cool", "jet", "hsv", "bone", "copper",
];

/// Names that run as statements without displaying a value (`colorbar`,
/// `text(...)`, `drawnow`, ...).
pub(in crate::engine) fn gobj_statement(name: &str) -> bool {
    matches!(
        name,
        "text"
            | "colormap"
            | "alphamap"
            | "colorbar"
            | "clim"
            | "caxis"
            | "yyaxis"
            | "drawnow"
            | "box"
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
            | "colororder"
            | "title"
            | "subtitle"
            | "xlabel"
            | "ylabel"
            | "zlabel"
            | "sgtitle"
    )
}
/// Names usable as bare words in expressions (`h = colorbar;`, `colormap(jet)`).
pub(in crate::engine) fn gobj_bare(name: &str) -> bool {
    (gobj_statement(name)
        && !matches!(
            name,
            "title" | "subtitle" | "xlabel" | "ylabel" | "zlabel" | "sgtitle"
        ))
        || MAP_FUNCTIONS.contains(&name)
}

pub(in crate::engine) fn handle_value(
    handle: GraphicsHandle,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Value> {
    Ok(Value::Container(
        GraphicsArray::scalar(handle, cancel)?.into(),
    ))
}
fn rgb_value(rgb: [f64; 3]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_slice(1, 3, &rgb)?))
}
fn scalar(v: f64) -> Value {
    Value::Matrix(Matrix::scalar(v))
}
fn on_off(on: bool) -> Value {
    Value::Text(if on { "on" } else { "off" }.into())
}
fn string(s: &str) -> Value {
    Value::Text(s.into())
}
fn map_value(rows: &[[f64; 3]]) -> Result<Value> {
    let mut data = Vec::with_capacity(rows.len() * 3);
    for c in 0..3 {
        data.extend(rows.iter().map(|r| r[c]));
    }
    Ok(Value::Matrix(Matrix::from_column_vec(rows.len(), 3, data)?))
}
fn map_rows(value: &Value, limit: usize, what: &str) -> Result<Vec<[f64; 3]>> {
    let Value::Matrix(m) = value else {
        return Err(format!(
            "{what} requires a real n-by-3 matrix with values in [0,1]"
        ));
    };
    if m.cols() != 3 || m.rows() == 0 || m.rows() > limit {
        return Err(format!(
            "{what} requires a real n-by-3 matrix with 1..{limit} rows"
        ));
    }
    let rows: Vec<[f64; 3]> = (0..m.rows())
        .map(|r| [m.get(r, 0), m.get(r, 1), m.get(r, 2)])
        .collect();
    if rows
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
    {
        return Err(format!("{what} values must be in [0,1]"));
    }
    Ok(rows)
}
fn real_vector(value: &Value, what: &str) -> Result<Vec<f64>> {
    match value {
        Value::Matrix(m) if m.rows() <= 1 || m.cols() <= 1 => Ok(m.data().to_vec()),
        Value::Logical(m) if m.rows() <= 1 || m.cols() <= 1 => {
            Ok(m.data().iter().map(|&v| f64::from(v)).collect())
        }
        _ => Err(format!("{what} requires a real vector")),
    }
}
pub(in crate::engine) fn real_scalar(value: &Value, what: &str) -> Result<f64> {
    let v = real_vector(value, what)?;
    if v.len() != 1 || !v[0].is_finite() {
        return Err(format!("{what} requires one finite real scalar"));
    }
    Ok(v[0])
}
fn limits(value: &Value, what: &str) -> Result<[f64; 2]> {
    let v = real_vector(value, what)?;
    if v.len() != 2 || !v.iter().all(|x| x.is_finite()) || v[0] >= v[1] {
        return Err(format!("{what} requires two finite increasing values"));
    }
    Ok([v[0], v[1]])
}
fn any_rgb(value: &Value, cancel: &std::sync::atomic::AtomicBool) -> Result<[f64; 3]> {
    if let Value::Text(_) = value {
        return Ok(super::super::plot_properties::color(value)?.map(|v| f64::from(v) / 255.));
    }
    axes_rgb(value, cancel)
}
fn on_off_arg(value: &Value, what: &str) -> Result<bool> {
    match value {
        Value::Text(s) if s.eq_ignore_ascii_case("on") => Ok(true),
        Value::Text(s) if s.eq_ignore_ascii_case("off") => Ok(false),
        Value::Logical(m) => m.as_scalar(),
        Value::Matrix(m) if m.as_scalar()? == 1. => Ok(true),
        Value::Matrix(m) if m.as_scalar()? == 0. => Ok(false),
        _ => Err(format!("{what} requires on or off")),
    }
}
fn axes_of<'a>(figure: &'a Figure, h: &GraphicsHandle) -> Result<&'a Axes> {
    figure
        .all_axes()
        .map(|(_, a)| a)
        .find(|a| {
            h.axes
                .as_ref()
                .is_some_and(|id| id.same_object(&a.identity))
        })
        .ok_or("Axes is no longer live".into())
}
fn axes_by_id<'a>(figure: &'a mut Figure, id: &ObjectIdentity) -> Result<&'a mut Axes> {
    figure
        .all_axes_mut()
        .find(|a| a.identity.same_object(id))
        .ok_or("Axes is no longer live".into())
}

/// Which axes-owned text a Text handle names.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextTarget {
    Object(usize),
    Title,
    Subtitle,
    XLabel,
    YLabel,
    ZLabel,
    RightYLabel,
    ColorbarLabel,
    ColorbarTitle,
}
fn text_target(axes: &Axes, id: &ObjectIdentity) -> Option<TextTarget> {
    let e = &axes.extras;
    if e.title_identity.same_object(id) {
        return Some(TextTarget::Title);
    }
    if e.subtitle_identity.same_object(id) {
        return Some(TextTarget::Subtitle);
    }
    if e.xlabel_identity.same_object(id) {
        return Some(TextTarget::XLabel);
    }
    if e.ylabel_identity.same_object(id) {
        return Some(TextTarget::YLabel);
    }
    if e.zlabel_identity.same_object(id) {
        return Some(TextTarget::ZLabel);
    }
    if e.yy
        .as_ref()
        .is_some_and(|y| y.right_label_identity.same_object(id))
    {
        return Some(TextTarget::RightYLabel);
    }
    if e.colorbar
        .as_ref()
        .is_some_and(|c| c.label_identity.same_object(id))
    {
        return Some(TextTarget::ColorbarLabel);
    }
    if e.colorbar
        .as_ref()
        .is_some_and(|c| c.title_identity.same_object(id))
    {
        return Some(TextTarget::ColorbarTitle);
    }
    e.texts
        .iter()
        .position(|t| t.identity.same_object(id))
        .map(TextTarget::Object)
}

/// MATLAB class name used in property errors.
fn class_name(h: &GraphicsHandle, figure: Option<&Figure>) -> &'static str {
    match h.kind() {
        GraphicsKind::Placeholder => "GraphicsPlaceholder",
        GraphicsKind::Root => "Root",
        GraphicsKind::Figure => "Figure",
        GraphicsKind::Axes => "Axes",
        GraphicsKind::TextBox => "TextBox",
        GraphicsKind::Annotation => "Annotation",
        GraphicsKind::Legend => "Legend",
        GraphicsKind::Baseline => "Baseline",
        GraphicsKind::Text => "Text",
        GraphicsKind::Colorbar => "ColorBar",
        GraphicsKind::Image => "Image",
        GraphicsKind::Volume => "Volume",
        GraphicsKind::SliceViewer => "SliceViewer",
        GraphicsKind::OrthosliceViewer => "OrthosliceViewer",
        GraphicsKind::TiledLayout => "TiledChartLayout",
        GraphicsKind::NumericRuler => "NumericRuler",
        GraphicsKind::ConstantLine => "ConstantLine",
        GraphicsKind::Rectangle => "Rectangle",
        GraphicsKind::UiComponent => "Graphics",
        GraphicsKind::Series => {
            let kind = figure.and_then(|f| {
                axes_of(f, h).ok().and_then(|a| {
                    a.series
                        .iter()
                        .find(|s| s.identity.same_object(&h.object))
                        .map(|s| s.kind)
                })
            });
            match kind {
                Some(crate::plot::PlotKind::Scatter) => "Scatter",
                Some(crate::plot::PlotKind::Bar) => "Bar",
                Some(crate::plot::PlotKind::Stairs) => "Stair",
                Some(crate::plot::PlotKind::Stem) => "Stem",
                Some(crate::plot::PlotKind::Patch) => "Patch",
                _ => "Line",
            }
        }
    }
}
/// Documented property names per class (lower case). A name outside this
/// list is "unrecognized"; a listed name that MatRust cannot query yet keeps
/// the more specific message from get/set.
pub(super) const ROOT_PROPERTIES: &[&str] = &[
    "Type",
    "Parent",
    "Children",
    "Tag",
    "UserData",
    "ScreenSize",
    "MonitorPositions",
    "ScreenPixelsPerInch",
    "Units",
    "CurrentFigure",
    "ShowHiddenHandles",
];

fn known_property(class: &str, name: &str) -> bool {
    if class == "Root" {
        return ROOT_PROPERTIES
            .iter()
            .any(|prop| prop.eq_ignore_ascii_case(name));
    }
    const COMMON: &[&str] = &[
        "type",
        "parent",
        "children",
        "visible",
        "tag",
        "userdata",
        "handlevisibility",
    ];
    let specific: &[&str] = match class {
        "Figure" => &[
            "name",
            "number",
            "color",
            "colormap",
            "currentaxes",
            "theme",
            "thememode",
            "colormode",
            "inverthardcopy",
            "position",
            "innerposition",
            "outerposition",
            "units",
            "numbertitle",
            "menubar",
            "toolbar",
            "windowstyle",
            "windowstate",
            "resize",
            "paperpositionmode",
            "renderer",
        ],
        "Axes" => &[
            "xlim",
            "ylim",
            "zlim",
            "xlimmode",
            "ylimmode",
            "zlimmode",
            "xscale",
            "yscale",
            "zscale",
            "title",
            "subtitle",
            "xlabel",
            "ylabel",
            "zlabel",
            "fontsize",
            "fontname",
            "fontweight",
            "colororder",
            "colormap",
            "clim",
            "climmode",
            "color",
            "xcolor",
            "ycolor",
            "zcolor",
            "gridcolor",
            "gridalpha",
            "xcolormode",
            "ycolormode",
            "gridcolormode",
            "gridalphamode",
            "xgrid",
            "ygrid",
            "zgrid",
            "xminorgrid",
            "yminorgrid",
            "zminorgrid",
            "box",
            "nextplot",
            "position",
            "innerposition",
            "outerposition",
            "positionconstraint",
            "activepositionproperty",
            "units",
            "yaxislocation",
            "xaxislocation",
            "xtick",
            "ytick",
            "ztick",
            "xticklabel",
            "yticklabel",
            "zticklabel",
            "xtickmode",
            "ytickmode",
            "ztickmode",
            "xticklabelmode",
            "yticklabelmode",
            "zticklabelmode",
            "xticklabelrotation",
            "yticklabelrotation",
            "zticklabelrotation",
            "tickdir",
            "ticklength",
            "layer",
            "zlimmode",
            "xaxis",
            "yaxis",
            "zaxis",
            "ticklabelinterpreter",
            "xdir",
            "ydir",
            "linewidth",
            "legend",
            "colorscale",
            "alim",
            "dataaspectratio",
            "plotboxaspectratio",
            "view",
            "layout",
        ],
        "NumericRuler" => &[
            "tickvalues",
            "tickvaluesmode",
            "ticklabels",
            "ticklabelsmode",
            "ticklabelrotation",
            "limits",
            "limitsmode",
            "exponent",
            "ticklabelformat",
            "categories",
            "color",
            "label",
        ],
        "Text" => &[
            "string",
            "position",
            "color",
            "fontsize",
            "fontweight",
            "fontname",
            "fontangle",
            "horizontalalignment",
            "verticalalignment",
            "rotation",
            "units",
            "interpreter",
            "backgroundcolor",
            "edgecolor",
            "margin",
            "extent",
            "clipping",
        ],
        "TiledChartLayout" => &[
            "gridsize",
            "tilespacing",
            "padding",
            "tilearrangement",
            "title",
            "subtitle",
            "xlabel",
            "ylabel",
        ],
        "ColorBar" => &[
            "limits",
            "label",
            "title",
            "location",
            "orientation",
            "fontsize",
            "ticks",
            "ticklabels",
            "color",
            "box",
            "direction",
            "position",
            "units",
        ],
        "Volume" => &[
            "data",
            "renderingstyle",
            "colormap",
            "alphamap",
            "datalimits",
            "isosurfacevalue",
            "isosurfacecolor",
            "sliceplanevalues",
            "transformation",
            "overlaydata",
            "overlayrenderingstyle",
            "overlaycolormap",
            "overlayalphamap",
            "overlaydisplayrange",
        ],
        "ConstantLine" => &[
            "value",
            "interceptaxis",
            "color",
            "linestyle",
            "linewidth",
            "alpha",
            "label",
            "labelhorizontalalignment",
            "labelverticalalignment",
            "displayname",
        ],
        "Rectangle" => &[
            "position",
            "curvature",
            "facecolor",
            "edgecolor",
            "linewidth",
            "linestyle",
        ],
        "Annotation" => &[
            "x",
            "y",
            "position",
            "color",
            "edgecolor",
            "linewidth",
            "linestyle",
            "headstyle",
            "headlength",
            "headwidth",
            "string",
            "fontsize",
            "facecolor",
            "facealpha",
            "units",
        ],
        "TextBox" => &[
            "string",
            "position",
            "units",
            "interpreter",
            "edgecolor",
            "backgroundcolor",
            "fitboxtotext",
            "verticalalignment",
            "horizontalalignment",
            "fontweight",
            "fontsize",
            "color",
        ],
        _ => &[
            "xdata",
            "ydata",
            "zdata",
            "color",
            "linestyle",
            "linewidth",
            "marker",
            "markersize",
            "markeredgecolor",
            "markerfacecolor",
            "markerfacealpha",
            "markeredgealpha",
            "displayname",
            "facecolor",
            "edgecolor",
            "barwidth",
            "barlayout",
            "basevalue",
            "baseline",
            "showbaseline",
            "sizedata",
            "cdata",
            "cdatamapping",
            "xdatamode",
            "seriesindex",
            "facealpha",
            "edgealpha",
            "vertices",
            "faces",
            "facevertexcdata",
            "groupwidth",
            "string",
            "location",
            "orientation",
            "box",
            "fontsize",
        ],
    };
    COMMON.contains(&name)
        || specific.contains(&name)
        || (class == "Patch" && matches!(name, "facevertexcdata" | "vertexnormals"))
        || (class == "Legend" && matches!(name, "fontweight" | "fontangle"))
}

impl Engine {
    fn gobj_class(&self, h: &GraphicsHandle) -> &'static str {
        if h.kind() == GraphicsKind::UiComponent {
            return super::super::ui_components::class_name(self, h);
        }
        let figure = self
            .locate_graphics(h)
            .ok()
            .and_then(|l| self.graphics_figure(l.figure).ok());
        class_name(h, figure)
    }
    /// `isprop(h, name)` for one handle: the documented property table of its
    /// class, case-insensitive like get and set.
    pub(in crate::engine) fn gobj_is_property(&self, h: &GraphicsHandle, name: &str) -> bool {
        if h.kind() == GraphicsKind::UiComponent {
            return super::super::ui_components::is_property(self, h, name);
        }
        if h.kind() == GraphicsKind::Axes && self.ui.is_uiaxes(&h.object) {
            return super::super::ui_components::is_property(self, h, name)
                || known_property("Axes", &name.to_ascii_lowercase());
        }
        known_property(self.gobj_class(h), &name.to_ascii_lowercase())
            || (name.eq_ignore_ascii_case("facelighting")
                && matches!(self.surface_get(h, name), Ok(Some(_))))
    }
    fn gobj_unrecognized(&mut self, h: &GraphicsHandle, name: &str, error: String) -> String {
        let class = self.gobj_class(h);
        let lower = name.to_ascii_lowercase();
        if self.gobj_is_property(h, name)
            || (class == "Root" && (lower.starts_with("default") || lower.starts_with("factory")))
        {
            return error;
        }
        self.raise(
            format!("Unrecognized property '{name}' for class '{class}'."),
            "MATLAB:class:InvalidProperty".into(),
        )
    }
    fn gobj_get_named(&mut self, h: &GraphicsHandle, name: &str) -> Result<Value> {
        if h.kind() == GraphicsKind::UiComponent {
            return super::super::ui_components::get_property(self, h, &[Value::Text(name.into())]);
        }
        if h.kind() == GraphicsKind::Axes
            && self.ui.is_uiaxes(&h.object)
            && super::super::ui_components::is_uiaxes_ui_property(name)
        {
            return super::super::ui_components::get_property(self, h, &[Value::Text(name.into())]);
        }
        if h.kind() == GraphicsKind::Root {
            return match self.get_root_property(&[Value::Text(name.into())]) {
                Ok(value) => Ok(value),
                Err(error) => Err(self.gobj_unrecognized(h, name, error)),
            };
        }
        let located = self.locate_graphics(h)?;
        match self.get_graphics_property(h, located, &[Value::Text(name.into())]) {
            Ok(value) => Ok(value),
            Err(error) => Err(self.gobj_unrecognized(h, name, error)),
        }
    }
    fn gobj_set_named(&mut self, h: &GraphicsHandle, name: &str, value: Value) -> Result<()> {
        if h.kind() == GraphicsKind::UiComponent {
            return super::super::ui_components::set_properties(
                self,
                h,
                &[Value::Text(name.into()), value],
            );
        }
        if h.kind() == GraphicsKind::Axes
            && self.ui.is_uiaxes(&h.object)
            && super::super::ui_components::is_uiaxes_ui_property(name)
        {
            return super::super::ui_components::set_properties(
                self,
                h,
                &[Value::Text(name.into()), value],
            );
        }
        if h.kind() == GraphicsKind::Root {
            let args = self.text_arguments(vec![Value::Text(name.into()), value])?;
            return match self.set_root_properties(&args) {
                Ok(()) => Ok(()),
                Err(error) => Err(self.gobj_unrecognized(h, name, error)),
            };
        }
        let located = self.locate_graphics(h)?;
        let raw = [Value::Text(name.into()), value];
        let mut args = self.text_arguments(raw.to_vec())?;
        if h.kind() == GraphicsKind::Text {
            super::preserve_string_property_values(&raw, &mut args);
        }
        match self.set_graphics_properties(h, located, &args) {
            Ok(()) => Ok(()),
            Err(error) => Err(self.gobj_unrecognized(h, name, error)),
        }
    }

    /// `h.Prop` for every handle in `h` (a comma-separated list for arrays).
    pub(in crate::engine) fn gobj_field_values(
        &mut self,
        handles: &GraphicsArray,
        name: &str,
    ) -> Result<Vec<Value>> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(handles.len())
            .map_err(|_| "Cannot allocate graphics property list")?;
        for i in 0..handles.len() {
            self.check_cancel()?;
            let h = handles.get_linear(i)?.clone();
            values.push(self.gobj_get_named(&h, name)?);
        }
        Ok(values)
    }

    /// `h.Prop = v`, `h.Prop(i) = v`, `h.Child.Prop = v` and `h(k).Prop = v`.
    pub(in crate::engine) fn gobj_assign(
        &mut self,
        handles: &GraphicsArray,
        steps: &[PathStep],
        value: &Array,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<Option<Array>> {
        let Some((step, rest)) = steps.split_first() else {
            return Ok(None);
        };
        let name = match step {
            PathStep::Field(name) => name.clone(),
            PathStep::Dynamic(expr) => {
                let value = self.eval(expr, None, out)?;
                match self.text_arguments(vec![value])?.remove(0) {
                    Value::Text(s) => s,
                    _ => return Err("Dynamic property names must be character text".into()),
                }
            }
            PathStep::Paren(args) if !rest.is_empty() => {
                let selected =
                    self.index_container(&Array::Graphics(handles.clone()), args, false, out)?;
                let Value::Container(Array::Graphics(selected)) = selected else {
                    return Err("Graphics handle indexing did not produce handles".into());
                };
                self.gobj_assign(&selected, rest, value, deletion, out)?;
                return Ok(Some(Array::Graphics(handles.clone())));
            }
            _ => return Ok(None),
        };
        if handles.len() != 1 {
            return Err(
                "Dot assignment needs one graphics handle; use set(h, Name, Value) for an array of handles"
                    .into(),
            );
        }
        let h = handles.scalar_handle()?.clone();
        if rest.is_empty() {
            self.gobj_set_named(&h, &name, Value::from(value.clone()))?;
        } else {
            let current = self.gobj_get_named(&h, &name)?;
            if let Value::Container(Array::Graphics(child)) = &current {
                self.gobj_assign(child, rest, value, deletion, out)?;
            } else {
                let current = current.array(&self.cancel)?;
                let updated =
                    self.assign_step(Some(current), rest, value.clone(), deletion, out)?;
                self.gobj_set_named(&h, &name, Value::from(updated))?;
            }
        }
        Ok(Some(Array::Graphics(handles.clone())))
    }

    /// Extended `get` queries; None leaves the query to the original code.
    pub(super) fn gobj_get(
        &self,
        h: &GraphicsHandle,
        located: Located,
        name: &Value,
    ) -> Result<Option<Value>> {
        let name = text(name)?.to_ascii_lowercase();
        let figure = self.graphics_figure(located.figure)?;
        let cancel = &self.cancel;
        let handle_value = |handle: GraphicsHandle| -> Result<Value> {
            Ok(Value::Container(
                GraphicsArray::scalar(handle, cancel)?.into(),
            ))
        };
        if h.kind() == GraphicsKind::Figure {
            return Ok(Some(match name.as_str() {
                "parent" => handle_value(GraphicsHandle::root())?,
                "colormap" => map_value(&figure.axes.extras.effective_colormap())?,
                "children" => {
                    let handles = figure
                        .all_axes()
                        .map(|(_, a)| Arc::new(GraphicsHandle::axes(&figure.identity, &a.identity)))
                        .collect::<Vec<_>>();
                    let n = handles.len();
                    Value::Container(GraphicsArray::from_shared(n, 1, handles, cancel)?.into())
                }
                "visible" => on_off(figure.visible && !self.is_figure_hidden(&figure.identity)),
                _ => return Ok(None),
            }));
        }
        if h.kind() == GraphicsKind::TiledLayout {
            let layout = figure
                .layout
                .as_ref()
                .filter(|l| l.tiled.is_some() && l.identity.same_object(&h.object))
                .ok_or("TiledChartLayout is no longer live")?;
            let grid = layout.tiled.unwrap();
            return Ok(Some(match name.as_str() {
                "type" => string("tiledlayout"),
                "parent" => handle_value(GraphicsHandle::figure(&figure.identity))?,
                "children" => {
                    let handles = if figure.empty_layout() {
                        Vec::new()
                    } else {
                        figure
                            .all_axes()
                            .map(|(_, a)| {
                                Arc::new(GraphicsHandle::axes(&figure.identity, &a.identity))
                            })
                            .collect()
                    };
                    let n = handles.len();
                    Value::Container(GraphicsArray::from_shared(n, 1, handles, cancel)?.into())
                }
                "gridsize" => Value::Matrix(Matrix::from_column_slice(
                    1,
                    2,
                    &[grid.rows as f64, grid.columns as f64],
                )?),
                "tilespacing" => string(layout.tile_spacing.as_str()),
                "padding" => string(layout.padding.as_str()),
                "tilearrangement" => string(match grid.arrangement {
                    crate::plot::Arrangement::Fixed => "fixed",
                    crate::plot::Arrangement::Flow => "flow",
                    crate::plot::Arrangement::Vertical => "vertical",
                    crate::plot::Arrangement::Horizontal => "horizontal",
                }),
                "title" => handle_value(GraphicsHandle::layout_text(
                    &figure.identity,
                    &layout.title_identity,
                ))?,
                "subtitle" => handle_value(GraphicsHandle::layout_text(
                    &figure.identity,
                    &layout.subtitle_identity,
                ))?,
                "xlabel" => handle_value(GraphicsHandle::layout_text(
                    &figure.identity,
                    &layout.xlabel_identity,
                ))?,
                "ylabel" => handle_value(GraphicsHandle::layout_text(
                    &figure.identity,
                    &layout.ylabel_identity,
                ))?,
                _ => return Ok(None),
            }));
        }
        if h.kind() == GraphicsKind::Text && h.axes.is_none() {
            let layout = figure
                .layout
                .as_ref()
                .ok_or("Text object is no longer live")?;
            let appearance = figure.resolve_appearance(self.graphics_theme_context);
            let rgb = appearance.shared_title.map(|v| f64::from(v) / 255.);
            let is_subtitle = h.object.same_object(&layout.subtitle_identity);
            if is_subtitle
                && let Some(value) = subtitle_property(&layout.subtitle, &name, rgb, cancel)?
            {
                return Ok(Some(value));
            }
            let (text_str, index) = if is_subtitle {
                (&layout.subtitle.string, 0)
            } else if h.object.same_object(&layout.title_identity) {
                (&layout.title, 0)
            } else if h.object.same_object(&layout.xlabel_identity) {
                (&layout.xlabel, 1)
            } else if h.object.same_object(&layout.ylabel_identity) {
                (&layout.ylabel, 2)
            } else {
                return Err("Text object is no longer live".into());
            };
            return Ok(Some(match name.as_str() {
                "type" => string("text"),
                "parent" => {
                    if layout.tiled.is_some() {
                        handle_value(GraphicsHandle::tiled_layout(
                            &figure.identity,
                            &layout.identity,
                        ))?
                    } else {
                        handle_value(GraphicsHandle::figure(&figure.identity))?
                    }
                }
                "string" => {
                    if let Some(lines) = &layout.text_lines[index] {
                        super::super::graphics_text_value(lines, &self.cancel)?
                    } else {
                        string(text_str)
                    }
                }
                "color" => rgb_value(rgb)?,
                "fontsize" if index == 0 => scalar(layout.title_style.font_size.unwrap_or(12.)),
                "fontsize" => scalar(layout.label_styles[index - 1].font_size.unwrap_or(12.)),
                "fontweight" if index == 0 => {
                    string(font_weight_name(layout.title_style.bold.unwrap_or(true)))
                }
                "fontweight" => string(font_weight_name(
                    layout.label_styles[index - 1].bold.unwrap_or(false),
                )),
                "fontangle" if index == 0 => {
                    string(font_angle_name(layout.title_style.italic.unwrap_or(false)))
                }
                "fontangle" => string(font_angle_name(
                    layout.label_styles[index - 1].italic.unwrap_or(false),
                )),
                "interpreter" => string(layout.text_interpreters[index].as_str()),
                "visible" => on_off(true),
                _ => return Ok(None),
            }));
        }
        if h.kind() == GraphicsKind::SliceViewer {
            let sv = figure
                .slice_viewers
                .iter()
                .find(|v| v.identity.same_object(&h.object))
                .ok_or("SliceViewer is no longer live")?;
            let parent = handle_value(GraphicsHandle::figure(&figure.identity))?;
            return Ok(Some(crate::engine::volume::slice_viewer::slice_viewer_get(
                sv, &name, parent,
            )?));
        }
        if h.kind() == GraphicsKind::OrthosliceViewer {
            let ov = figure
                .orthoslice_viewers
                .iter()
                .find(|v| v.identity.same_object(&h.object))
                .ok_or("OrthosliceViewer is no longer live")?;
            let parent = handle_value(GraphicsHandle::figure(&figure.identity))?;
            return Ok(Some(
                crate::engine::volume::slice_viewer::orthoslice_viewer_get(ov, &name, parent)?,
            ));
        }
        let axes = if h.kind() == GraphicsKind::Volume {
            figure
                .all_axes()
                .find_map(|(pos, axes)| (Some(pos) == located.position).then_some(axes))
                .ok_or("Volume axes is no longer live")?
        } else {
            axes_of(figure, h)?
        };
        let extras = &axes.extras;
        let child = |id: &ObjectIdentity, kind| {
            handle_value(GraphicsHandle::axes_object(
                &figure.identity,
                &axes.identity,
                id,
                kind,
            ))
        };
        match h.kind() {
            GraphicsKind::Text => {
                let target = text_target(axes, &h.object).ok_or("Text object is no longer live")?;
                let appearance = figure.resolve_appearance(self.graphics_theme_context);
                let colors = axes.resolve_colors(appearance);
                let label_rgb = |rgb: [u8; 3]| rgb.map(|v| f64::from(v) / 255.);
                let default_size = extras.font_size.unwrap_or(10.);
                if target == TextTarget::Subtitle
                    && let Some(value) = subtitle_property(
                        &extras.subtitle,
                        &name,
                        label_rgb(colors.title),
                        &self.cancel,
                    )?
                {
                    return Ok(Some(value));
                }
                let value = match (target, name.as_str()) {
                    (_, "type") => string("text"),
                    (TextTarget::ColorbarLabel | TextTarget::ColorbarTitle, "parent") => {
                        handle_value(GraphicsHandle::axes_object(
                            &figure.identity,
                            &axes.identity,
                            &extras.colorbar.as_ref().unwrap().identity,
                            GraphicsKind::Colorbar,
                        ))?
                    }
                    (_, "parent") => {
                        handle_value(GraphicsHandle::axes(&figure.identity, &axes.identity))?
                    }
                    (_, "units") => string("data"),
                    (target, "interpreter") => string(match target {
                        TextTarget::Object(i) => extras.texts[i].interpreter.as_str(),
                        TextTarget::Title => extras.label_interpreters[0].as_str(),
                        TextTarget::XLabel => extras.label_interpreters[1].as_str(),
                        TextTarget::YLabel => extras.label_interpreters[2].as_str(),
                        TextTarget::ZLabel => extras.label_interpreters[3].as_str(),
                        _ => "tex",
                    }),
                    (TextTarget::Object(i), _) => {
                        let t = &extras.texts[i];
                        match name.as_str() {
                            "string" => if let Some(lines) = &t.string_lines { super::super::graphics_text_value(lines, &self.cancel)? } else { string(&t.string) },
                            "position" => Value::Matrix(Matrix::from_column_slice(1, 3, &t.position)?),
                            "color" => rgb_value(t.color.unwrap_or_else(|| label_rgb(appearance.text)))?,
                            "fontsize" => scalar(t.font_size),
                            "fontweight" => string(if t.bold { "bold" } else { "normal" }),
                            "fontangle" => string(font_angle_name(t.italic)),
                            "horizontalalignment" => string(crate::plot::horizontal_name(t.horizontal)),
                            "verticalalignment" => string(t.vertical.as_str()),
                            "rotation" => scalar(t.rotation),
                            "visible" => on_off(t.visible),
                            _ => return Err("Supported text queries: Type, Parent, String, Position, Color, FontSize, FontWeight, HorizontalAlignment, VerticalAlignment, Rotation, Visible, Units".into()),
                        }
                    }
                    (target, _) => {
                        let (text_value, rgb, size) = match target {
                            TextTarget::Title => (
                                axes.title.as_str(),
                                label_rgb(colors.title),
                                extras.title_style.font_size.unwrap_or(default_size * 1.1),
                            ),
                            TextTarget::Subtitle => (
                                axes.extras.subtitle.string.as_str(),
                                label_rgb(colors.title),
                                default_size,
                            ),
                            TextTarget::XLabel => (
                                axes.xlabel.as_str(),
                                label_rgb(colors.xlabel),
                                extras.label_styles[0]
                                    .font_size
                                    .unwrap_or(default_size * 1.1),
                            ),
                            TextTarget::YLabel => (
                                axes.ylabel.as_str(),
                                label_rgb(colors.ylabel),
                                extras.label_styles[1]
                                    .font_size
                                    .unwrap_or(default_size * 1.1),
                            ),
                            TextTarget::ZLabel => (
                                axes.zlabel.as_str(),
                                label_rgb(colors.ylabel),
                                extras.label_styles[2]
                                    .font_size
                                    .unwrap_or(default_size * 1.1),
                            ),
                            TextTarget::RightYLabel => {
                                let yy = extras.yy.as_ref().unwrap();
                                (
                                    yy.right_label.as_str(),
                                    yy.right_color,
                                    yy.right_label_style.font_size.unwrap_or(default_size * 1.1),
                                )
                            }
                            TextTarget::ColorbarLabel => {
                                let bar = extras.colorbar.as_ref().unwrap();
                                (bar.label.as_str(), label_rgb(appearance.text), default_size)
                            }
                            TextTarget::ColorbarTitle => {
                                let bar = extras.colorbar.as_ref().unwrap();
                                (bar.title.as_str(), label_rgb(appearance.text), default_size)
                            }
                            TextTarget::Object(_) => unreachable!(),
                        };
                        match name.as_str() {
                            "string" => {
                                let index = match target { TextTarget::Title => Some(0), TextTarget::XLabel => Some(1), TextTarget::YLabel => Some(2), TextTarget::ZLabel => Some(3), _ => None };
                                if let Some(lines) = index.and_then(|i| extras.label_lines[i].as_ref()) {
                                    super::super::graphics_text_value(lines, &self.cancel)?
                                } else { string(text_value) }
                            },
                            "color" => rgb_value(rgb)?,
                            "fontsize" => scalar(size),
                            "visible" => on_off(figure.visible && !self.is_figure_hidden(&figure.identity)),
                            "fontweight" => string(font_weight_name(match target {
                                TextTarget::Title => extras.title_style.bold.unwrap_or(true),
                                TextTarget::XLabel => extras.label_styles[0].bold.unwrap_or(false),
                                TextTarget::YLabel => extras.label_styles[1].bold.unwrap_or(false),
                                TextTarget::ZLabel => extras.label_styles[2].bold.unwrap_or(false),
                                TextTarget::RightYLabel => extras.yy.as_ref().unwrap().right_label_style.bold.unwrap_or(false),
                                _ => false,
                            })),
                            "fontangle" => string(font_angle_name(match target {
                                TextTarget::Title => extras.title_style.italic.unwrap_or(false),
                                TextTarget::XLabel => extras.label_styles[0].italic.unwrap_or(false),
                                TextTarget::YLabel => extras.label_styles[1].italic.unwrap_or(false),
                                TextTarget::ZLabel => extras.label_styles[2].italic.unwrap_or(false),
                                TextTarget::RightYLabel => extras.yy.as_ref().unwrap().right_label_style.italic.unwrap_or(false),
                                _ => false,
                            })),
                            "horizontalalignment" => string("center"),
                            _ => return Err("Supported title/label queries: Type, Parent, String, Color, FontSize, FontWeight, Visible".into()),
                        }
                    }
                };
                Ok(Some(value))
            }
            GraphicsKind::Image => {
                let image = extras
                    .images
                    .iter()
                    .find(|i| i.identity.same_object(&h.object))
                    .ok_or("Image is no longer live")?;
                let parent = handle_value(GraphicsHandle::axes(&figure.identity, &axes.identity))?;
                Ok(Some(super::image_commands::image_get(
                    self, image, &name, parent,
                )?))
            }
            GraphicsKind::Volume => {
                let volume = extras
                    .volumes
                    .iter()
                    .find(|v| v.identity.same_object(&h.object))
                    .ok_or("Volume is no longer live")?;
                let parent = handle_value(GraphicsHandle::axes(&figure.identity, &axes.identity))?;
                Ok(Some(crate::engine::volume::show::volume_get(
                    self, volume, &name, parent,
                )?))
            }
            GraphicsKind::ConstantLine => {
                let line = extras
                    .constant_lines
                    .iter()
                    .find(|l| l.identity.same_object(&h.object))
                    .ok_or("ConstantLine is no longer live")?;
                let parent = handle_value(GraphicsHandle::axes(&figure.identity, &axes.identity))?;
                Ok(Some(super::shapes_commands::constant_line_get(
                    line, &name, parent,
                )?))
            }
            GraphicsKind::Rectangle => {
                let rect = extras
                    .rectangles
                    .iter()
                    .find(|r| r.identity.same_object(&h.object))
                    .ok_or("Rectangle is no longer live")?;
                let parent = handle_value(GraphicsHandle::axes(&figure.identity, &axes.identity))?;
                Ok(Some(super::shapes_commands::rectangle_get(
                    rect, &name, parent,
                )?))
            }
            GraphicsKind::Colorbar => {
                let bar = extras
                    .colorbar
                    .as_ref()
                    .ok_or("Colorbar is no longer live")?;
                Ok(Some(match name.as_str() {
                    "type" => string("colorbar"),
                    "parent" => handle_value(GraphicsHandle::figure(&figure.identity))?,
                    "limits" => Value::Matrix(Matrix::from_column_slice(1, 2, &axes.color_limits())?),
                    "label" => child(&bar.label_identity, GraphicsKind::Text)?,
                    "title" => child(&bar.title_identity, GraphicsKind::Text)?,
                    "location" => string(bar.location.as_str()),
                    "orientation" => string(if bar.location.horizontal() { "horizontal" } else { "vertical" }),
                    "visible" => on_off(bar.visible),
                    "fontsize" => scalar(extras.colorbar_font_size()),
                    _ => return Err("Supported colorbar queries: Type, Parent, Limits, Label, Title, Location, Orientation, Visible, FontSize".into()),
                }))
            }
            GraphicsKind::NumericRuler => {
                let axis = extras
                    .ruler_index(&h.object)
                    .ok_or("NumericRuler is no longer live")?;
                Ok(Some(super::super::graphics_ticks::ruler_get_property(
                    axes,
                    figure,
                    axis,
                    &name,
                    self.graphics_theme_context,
                    &self.cancel,
                )?))
            }
            GraphicsKind::Axes => {
                let yy_right = extras.yy.as_ref().filter(|y| y.active_right);
                Ok(Some(match name.as_str() {
                    "tag" => string(&extras.tag),
                    "visible" => on_off(!extras.axis_off),
                    "plotboxaspectratiomode" => string(if extras.square_plot_box {
                        "manual"
                    } else {
                        "auto"
                    }),
                    "plotboxaspectratio" => {
                        let ratio = if extras.square_plot_box {
                            [1.; 3]
                        } else {
                            let position = figure
                                .all_axes()
                                .find(|(_, a)| a.identity.same_object(&axes.identity))
                                .ok_or("Axes is no longer live")?
                                .0;
                            let (_, geometry) = figure
                                .resolved_axes_geometry()?
                                .into_iter()
                                .find(|(p, _)| *p == position)
                                .ok_or("Axes plot box is unavailable")?;
                            let canvas_height =
                                if figure.layout.as_ref().is_some_and(|l| !l.title.is_empty()) {
                                    524.
                                } else {
                                    560.
                                };
                            let width = 960. * geometry.data.width();
                            let height = canvas_height * geometry.data.height();
                            let side = width.min(height);
                            [width / side, height / side, 1.]
                        };
                        Value::Matrix(Matrix::from_column_slice(1, 3, &ratio)?)
                    }
                    "xaxis" => child(&extras.ruler_identities[0], GraphicsKind::NumericRuler)?,
                    "yaxis" => child(&extras.ruler_identities[1], GraphicsKind::NumericRuler)?,
                    "zaxis" => child(&extras.ruler_identities[2], GraphicsKind::NumericRuler)?,
                    "xtick" | "ytick" | "ztick" => {
                        let axis = match name.as_str() {
                            "xtick" => 0,
                            "ytick" => 1,
                            _ => 2,
                        };
                        let (ticks, _) = axes.effective_ticks(axis);
                        super::super::plot_time_axes::ruler_values_to_value(
                            &axes.extras.rulers[axis].kind,
                            &ticks,
                            &self.cancel,
                        )?
                    }
                    "xtickmode" | "ytickmode" | "ztickmode" => {
                        let axis = match name.as_str() {
                            "xtickmode" => 0,
                            "ytickmode" => 1,
                            _ => 2,
                        };
                        string(if extras.rulers[axis].ticks.is_some() {
                            "manual"
                        } else {
                            "auto"
                        })
                    }
                    "xticklabel" | "yticklabel" | "zticklabel" => {
                        let axis = match name.as_str() {
                            "xticklabel" => 0,
                            "yticklabel" => 1,
                            _ => 2,
                        };
                        let labels = match &extras.rulers[axis].tick_labels {
                            Some(labels) => labels.clone(),
                            None => axes.effective_tick_labels(axis),
                        };
                        super::super::graphics_ticks::labels_to_cell(&labels, &self.cancel)?
                    }
                    "xticklabelmode" | "yticklabelmode" | "zticklabelmode" => {
                        let axis = match name.as_str() {
                            "xticklabelmode" => 0,
                            "yticklabelmode" => 1,
                            _ => 2,
                        };
                        string(if extras.rulers[axis].tick_labels.is_some() {
                            "manual"
                        } else {
                            "auto"
                        })
                    }
                    "xticklabelrotation" | "yticklabelrotation" | "zticklabelrotation" => {
                        let axis = match name.as_str() {
                            "xticklabelrotation" => 0,
                            "yticklabelrotation" => 1,
                            _ => 2,
                        };
                        scalar(extras.rulers[axis].tick_label_rotation)
                    }
                    "tickdir" => string(extras.tick_dir.unwrap_or_default().as_str()),
                    "layer" => string(extras.layer.as_str()),
                    "ticklength" => {
                        let len = extras.tick_length.unwrap_or([0.01, 0.025]);
                        Value::Matrix(Matrix::from_column_slice(1, 2, &len)?)
                    }
                    "linewidth" => scalar(extras.line_width.unwrap_or(0.5)),
                    "ticklabelinterpreter" => string(extras.tick_label_interpreter.as_str()),
                    "title" => child(&extras.title_identity, GraphicsKind::Text)?,
                    "subtitle" => child(&extras.subtitle_identity, GraphicsKind::Text)?,
                    "xlabel" => child(&extras.xlabel_identity, GraphicsKind::Text)?,
                    "ylabel" => match yy_right {
                        Some(yy) => child(&yy.right_label_identity, GraphicsKind::Text)?,
                        None => child(&extras.ylabel_identity, GraphicsKind::Text)?,
                    },
                    "zlabel" => child(&extras.zlabel_identity, GraphicsKind::Text)?,
                    "view" => super::view_value(axes.view)?,
                    "zlim" => Value::Matrix(Matrix::from_column_slice(1, 2, &axes.zbounds())?),
                    "zlimmode" => string(if axes.zlim.is_some() {
                        "manual"
                    } else {
                        "auto"
                    }),
                    "zgrid" => on_off(axes.zgrid),
                    "box" => on_off(axes.box_outline),
                    "xminorgrid" => on_off(axes.minor_grid[0]),
                    "yminorgrid" => on_off(axes.minor_grid[1]),
                    "zminorgrid" => on_off(axes.minor_grid[2]),
                    "fontsize" => scalar(extras.font_size.unwrap_or(10.)),
                    "colororder" => {
                        if let Some(cycle) = axes.palette_cycle
                            && extras.color_order.is_none()
                        {
                            let theme =
                                figure.resolve_appearance(self.graphics_theme_context).theme;
                            let order = super::super::graphics_root::automatic_color_order(
                                cycle.version,
                                theme,
                            );
                            map_value(&order)?
                        } else {
                            map_value(&extras.effective_color_order())?
                        }
                    }
                    "colormap" => map_value(&extras.effective_colormap())?,
                    "clim" => Value::Matrix(Matrix::from_column_slice(1, 2, &axes.color_limits())?),
                    "climmode" => string(if extras.clim.is_some() {
                        "manual"
                    } else {
                        "auto"
                    }),
                    "yaxislocation" => string(if yy_right.is_some() { "right" } else { "left" }),
                    "ylim" if yy_right.is_some() => {
                        let (_, right) = axes.yy_ranges().unwrap();
                        Value::Matrix(Matrix::from_column_slice(1, 2, &right)?)
                    }
                    "ylim" if extras.yy.is_some() => {
                        let (left, _) = axes.yy_ranges().unwrap();
                        Value::Matrix(Matrix::from_column_slice(1, 2, &left)?)
                    }
                    "ycolor" if extras.yy.is_some() => {
                        let yy = extras.yy.as_ref().unwrap();
                        rgb_value(if yy.active_right {
                            yy.right_color
                        } else {
                            yy.left_color
                        })?
                    }
                    "children" => {
                        // Text overlays precede data children, including after native reload.
                        let mut handles = Vec::new();
                        for t in extras.texts.iter().rev() {
                            handles.push(Arc::new(GraphicsHandle::axes_object(
                                &figure.identity,
                                &axes.identity,
                                &t.identity,
                                GraphicsKind::Text,
                            )));
                        }
                        use crate::plot::{AxesDataChild, axes_data_children};
                        handles.extend(axes_data_children(axes).into_iter().map(|child| {
                            Arc::new(match child {
                                AxesDataChild::Series(i) => GraphicsHandle::series(
                                    &figure.identity,
                                    &axes.identity,
                                    &axes.series[i].identity,
                                ),
                                AxesDataChild::Volume(i) => {
                                    GraphicsHandle::volume(&extras.volumes[i].identity)
                                }
                            })
                        }));
                        let n = handles.len();
                        Value::Container(GraphicsArray::from_shared(n, 1, handles, cancel)?.into())
                    }
                    _ => return Ok(None),
                }))
            }
            GraphicsKind::Series => {
                let Some(s) = axes
                    .series
                    .iter()
                    .find(|s| s.identity.same_object(&h.object))
                else {
                    return Err("Series is no longer live".into());
                };
                let slice_level = s.slice_contour_level;
                let is_face_patch = s.kind == crate::plot::PlotKind::Patch && slice_level.is_none();
                Ok(Some(match name.as_str() {
                    "xdata" | "ydata" if !is_face_patch => {
                        let column = usize::from(name == "ydata");
                        let data: Vec<f64> = s.points.iter().map(|p| p[column]).collect();
                        super::super::plot_time_axes::ruler_values_to_value(
                            &axes.extras.rulers[column].kind,
                            &data,
                            cancel,
                        )?
                    }
                    // A 2-D line has empty ZData, as in MATLAB.
                    "zdata" if !is_face_patch => Value::Matrix(Matrix::from_column_vec(
                        usize::from(s.is_3d()),
                        s.z.len(),
                        s.z.to_vec(),
                    )?),
                    "cdata" if let Some(level) = slice_level => scalar(level),
                    "facecolor" if slice_level.is_some() => string("none"),
                    "edgecolor" if slice_level.is_some() => string("flat"),
                    "color"
                        if !matches!(
                            s.kind,
                            crate::plot::PlotKind::Patch | crate::plot::PlotKind::Bar
                        ) =>
                    {
                        let appearance = figure.resolve_appearance(self.graphics_theme_context);
                        rgb_value(appearance.series_color(s).map(|v| f64::from(v) / 255.))?
                    }
                    "marker"
                        if !matches!(
                            s.kind,
                            crate::plot::PlotKind::Patch | crate::plot::PlotKind::Bar
                        ) =>
                    {
                        let shape = if s.kind == crate::plot::PlotKind::Scatter || s.markers {
                            Some(s.effective_marker())
                        } else {
                            None
                        };
                        string(shape.map_or("none", |m| m.symbol()))
                    }
                    "markersize"
                        if !matches!(
                            s.kind,
                            crate::plot::PlotKind::Patch | crate::plot::PlotKind::Bar
                        ) =>
                    {
                        scalar(s.marker_size.unwrap_or(6.))
                    }
                    "sizedata" if s.kind == crate::plot::PlotKind::Scatter => {
                        match &s.marker_sizes {
                            Some(sizes) => Value::Matrix(Matrix::from_column_vec(
                                1,
                                sizes.len(),
                                sizes.to_vec(),
                            )?),
                            None => scalar(s.marker_size_points().powi(2)),
                        }
                    }
                    "cdata" if s.kind == crate::plot::PlotKind::Scatter => match &s.scatter_cdata {
                        Some(crate::plot::ScatterCData::Rgb(rgbs)) => {
                            let n = rgbs.len();
                            let mut data = Vec::with_capacity(n * 3);
                            for col in 0..3 {
                                for rgb in rgbs.iter() {
                                    data.push(f64::from(rgb[col]) / 255.);
                                }
                            }
                            Value::Matrix(Matrix::from_column_vec(n, 3, data)?)
                        }
                        Some(crate::plot::ScatterCData::Indexed(vals)) => {
                            Value::Matrix(Matrix::from_column_vec(vals.len(), 1, vals.to_vec())?)
                        }
                        None => {
                            let appearance = figure.resolve_appearance(self.graphics_theme_context);
                            rgb_value(appearance.series_color(s).map(|v| f64::from(v) / 255.))?
                        }
                    },
                    "markerfacecolor" if s.kind == crate::plot::PlotKind::Scatter => {
                        use crate::plot::MarkerColor;
                        match s.marker_face_color {
                            MarkerColor::None => string("none"),
                            MarkerColor::Flat => string("flat"),
                            MarkerColor::Auto => string("auto"),
                            MarkerColor::Rgb(rgb) => rgb_value(rgb.map(|v| f64::from(v) / 255.))?,
                            MarkerColor::Default => {
                                string(if s.marker_filled { "flat" } else { "none" })
                            }
                        }
                    }
                    "markeredgecolor" if s.kind == crate::plot::PlotKind::Scatter => {
                        use crate::plot::MarkerColor;
                        match s.marker_edge_color {
                            MarkerColor::None => string("none"),
                            MarkerColor::Default | MarkerColor::Flat | MarkerColor::Auto => {
                                string("flat")
                            }
                            MarkerColor::Rgb(rgb) => rgb_value(rgb.map(|v| f64::from(v) / 255.))?,
                        }
                    }
                    "markerfacealpha" if s.kind == crate::plot::PlotKind::Scatter => {
                        scalar(s.marker_face_alpha.unwrap_or(1.))
                    }
                    "markeredgealpha" if s.kind == crate::plot::PlotKind::Scatter => {
                        scalar(s.marker_edge_alpha.unwrap_or(1.))
                    }
                    _ => return Ok(None),
                }))
            }
            _ => Ok(None),
        }
    }

    /// Shared target resolution for the commands below: an optional leading
    /// axes/figure handle, else the current axes (created when none exists).
    fn gobj_target(&mut self, target: Option<&Value>) -> Result<(u32, Figure, ObjectIdentity)> {
        match target {
            Some(value) => {
                let h = handle(value)?;
                let located = self.locate_graphics(h)?;
                let figure = self.graphics_figure(located.figure)?.clone();
                let axes = match h.kind() {
                    GraphicsKind::Axes => h.axes.clone().unwrap(),
                    GraphicsKind::Figure => figure.axes.identity.clone(),
                    _ => return Err("Target must be an axes or figure handle".into()),
                };
                Ok((located.figure, figure, axes))
            }
            None => {
                let mut figure = self.figure.clone().unwrap_or_default();
                if self.figure.is_none() {
                    self.apply_figure_defaults(&mut figure)?;
                    self.apply_axes_defaults(&mut figure)?;
                }
                let axes = figure.axes.identity.clone();
                Ok((self.current_figure, figure, axes))
            }
        }
    }
    pub(super) fn gobj_publish(&mut self, id: u32, mut figure: Figure) -> Result<()> {
        for axes in figure.all_axes_mut() {
            super::super::plot_properties::refresh_scatter_clim(axes);
            super::super::plot_properties::refresh_scalar_patch_colors(axes);
        }
        // Colormap and CLim changes recolour the surfaces of the figure.
        self.surface_refresh(&mut figure, false)?;
        figure.validate()?;
        figure.prepare_legend_layout()?;
        self.check_cancel()?;
        if super::super::ui_components::is_uiaxes_fig_id(id) {
            self.publish_graphics_figure(id, figure);
            return Ok(());
        }
        if id == self.current_figure || self.figure.is_none() {
            if id != self.current_figure {
                self.select_figure(id)?;
            }
            self.figure = Some(figure);
        } else {
            self.publish_graphics_figure(id, figure);
        }
        Ok(())
    }

    /// Builtins of this module; None leaves the call to other dispatchers.
    pub(super) fn gobj_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let yy = self.figure.as_ref().and_then(|f| f.axes.extras.yy.as_ref());
        let yy_right_name = !matches!(args.first(), Some(Value::Container(Array::Graphics(_))))
            && match name {
                // Left-side ylim reports the left series only.
                "ylim" => yy.is_some(),
                "ylabel" => yy.is_some_and(|y| y.active_right),
                _ => false,
            };
        if !gobj_bare(name) && name != "ishandle" && !yy_right_name {
            return Ok(None);
        }
        self.check_cancel()?;
        let mut normalized = self.text_arguments(args.to_vec())?;
        if name == "text" {
            let offset = usize::from(matches!(
                args.first(),
                Some(Value::Container(Array::Graphics(_)))
            ));
            let numeric = |v: &Value| matches!(v, Value::Matrix(_) | Value::Logical(_));
            let index = offset
                + if args.get(offset + 2).is_some_and(numeric) {
                    3
                } else {
                    2
                };
            if matches!(args.get(index), Some(Value::QuotedString(_))) {
                normalized[index] = args[index].clone();
            }
        }
        let args = normalized;
        let leading = matches!(args.first(), Some(Value::Container(Array::Graphics(_))));
        let (target, rest) = if leading && name != "ishandle" {
            (Some(&args[0]), &args[1..])
        } else {
            (None, &args[..])
        };
        if MAP_FUNCTIONS.contains(&name) {
            let n = match args.as_slice() {
                [] => colormap::DEFAULT_ROWS,
                [n] => {
                    let n = real_scalar(n, name)?;
                    if n < 0. || n.fract() != 0. || n > crate::plot::MAX_COLORMAP_ROWS as f64 {
                        return Err(format!("{name}(n) requires an integer n in 0..4096"));
                    }
                    n as usize
                }
                _ => return Err(format!("Use {name} or {name}(n)")),
            };
            let rows = colormap::named(name, n)?.unwrap();
            if rows.is_empty() {
                return Ok(Some(Value::Matrix(Matrix::from_column_vec(
                    0,
                    3,
                    Vec::new(),
                )?)));
            }
            return map_value(&rows).map(Some);
        }
        match name {
            "ishandle" => self.gobj_ishandle(&args).map(Some),
            "drawnow" => {
                if args.len() > 1
                    || args.first().is_some_and(|a| {
                        !matches!(a, Value::Text(s) if matches!(s.to_ascii_lowercase().as_str(), "limitrate" | "nocallbacks" | "update" | "expose"))
                    })
                {
                    return Err("Use drawnow, or drawnow limitrate/nocallbacks/update/expose".into());
                }
                let skip_callbacks = matches!(
                    args.first(),
                    Some(Value::Text(s)) if matches!(
                        s.to_ascii_lowercase().as_str(),
                        "nocallbacks" | "update" | "expose"
                    )
                );
                self.gobj_flush_frame();
                if !skip_callbacks {
                    crate::engine::ui_components::drain_ui_events(self, out)?;
                }
                Ok(Some(Value::Void))
            }
            "text" => self.gobj_text(target, rest).map(Some),
            "colormap" => self.gobj_colormap(target, rest).map(Some),
            "alphamap" => crate::engine::volume::show::alphamap_call(self, &args, 1).map(Some),
            "colorbar" => self.gobj_colorbar(target, rest).map(Some),
            "clim" | "caxis" => self.gobj_clim(name, target, rest).map(Some),
            "yyaxis" => self.gobj_yyaxis(target, rest).map(Some),
            "box" => {
                let Some(target) = target else {
                    let mut out = crate::engine::Execution::default();
                    return self.graphics("box", rest.to_vec(), &mut out).map(Some);
                };
                let (id, mut figure, axes_id) = self.gobj_target(Some(target))?;
                let axes = axes_by_id(&mut figure, &axes_id)?;
                axes.box_outline = crate::engine::graphics::box_state(axes.box_outline, rest)?;
                self.gobj_publish(id, figure)?;
                Ok(Some(Value::Void))
            }
            "ylim" => {
                let (id, mut figure, axes_id) = self.gobj_target(None)?;
                let axes = axes_by_id(&mut figure, &axes_id)?;
                let right_side = axes.extras.yy.as_ref().unwrap().active_right;
                match rest {
                    [] => {
                        let (left, right) = axes.yy_ranges().unwrap();
                        let side = if right_side { right } else { left };
                        Ok(Some(Value::Matrix(Matrix::from_column_slice(1, 2, &side)?)))
                    }
                    [Value::Text(mode)] if mode.eq_ignore_ascii_case("auto") => {
                        if right_side {
                            axes.extras.yy.as_mut().unwrap().right_ylim = None;
                        } else {
                            axes.ylim = None;
                        }
                        self.gobj_publish(id, figure)?;
                        Ok(Some(Value::Void))
                    }
                    [value] => {
                        let v = limits(value, "ylim")?;
                        if right_side {
                            axes.extras.yy.as_mut().unwrap().right_ylim = Some(v);
                        } else {
                            axes.ylim = Some(v);
                        }
                        self.gobj_publish(id, figure)?;
                        Ok(Some(Value::Void))
                    }
                    _ => Err("Use ylim([ymin ymax]) or ylim('auto') for the right y-axis".into()),
                }
            }
            "ylabel" => {
                let (id, mut figure, axes_id) = self.gobj_target(None)?;
                let axes = axes_by_id(&mut figure, &axes_id)?;
                let [Value::Text(label), props @ ..] = rest else {
                    return Err("ylabel requires character text".into());
                };
                if props.len() > 10 || !props.len().is_multiple_of(2) {
                    return Err("ylabel expects a label and optional Name,Value pairs".into());
                }
                let yy = axes.extras.yy.as_mut().unwrap();
                for pair in props.as_chunks::<2>().0 {
                    let name = text(&pair[0])?.to_ascii_lowercase();
                    set_label_font_style(&mut yy.right_label_style, &name, &pair[1])?;
                }
                yy.right_label.clone_from(label);
                let identity = yy.right_label_identity.clone();
                let handle = GraphicsHandle::axes_object(
                    &figure.identity,
                    &axes_id,
                    &identity,
                    GraphicsKind::Text,
                );
                let result = Value::Container(GraphicsArray::scalar(handle, &self.cancel)?.into());
                self.gobj_publish(id, figure)?;
                Ok(Some(result))
            }
            _ => Ok(None),
        }
    }

    fn gobj_ishandle(&mut self, args: &[Value]) -> Result<Value> {
        let [value] = args else {
            return Err("ishandle requires one argument".into());
        };
        match value {
            Value::Container(Array::Graphics(h)) => {
                let mut values = Vec::with_capacity(h.len());
                for i in 0..h.len() {
                    self.check_cancel()?;
                    values.push(u8::from(self.locate_graphics(h.get_linear(i)?).is_ok()));
                }
                Ok(Value::Logical(LogicalMatrix::from_column_vec(
                    h.rows(),
                    h.cols(),
                    values,
                    &self.cancel,
                )?))
            }
            Value::Matrix(m) => {
                let ids = self.figure_ids();
                let values = m
                    .data()
                    .iter()
                    .map(|&v| {
                        u8::from(
                            v == 0. || (v.fract() == 0. && v >= 1. && ids.contains(&(v as u32))),
                        )
                    })
                    .collect();
                Ok(Value::Logical(LogicalMatrix::from_column_vec(
                    m.rows(),
                    m.cols(),
                    values,
                    &self.cancel,
                )?))
            }
            _ => Ok(Value::Logical(LogicalMatrix::from_column_vec(
                1,
                1,
                vec![0],
                &self.cancel,
            )?)),
        }
    }

    /// Send the current figures and UI windows to a host that redraws during a run.
    pub(in crate::engine) fn gobj_flush_frame(&mut self) {
        let Some(host) = self.host_io.clone() else {
            return;
        };
        let has_ui = !self.ui.figures.is_empty()
            || !self.ui.classic_fig_children.is_empty()
            || !self.ui.pending_alert_figs.is_empty()
            || !self.ui.active_confirm_figs.is_empty();
        if self.figure.is_none() && self.other_figures.is_empty() && !has_ui {
            return;
        }
        let frame = crate::engine::GraphicsFrame {
            figures: self.figures().map(|(id, f)| (id, f.clone())).collect(),
            figure: self.figure.clone(),
            current: self.current_figure,
            ui: crate::engine::ui_components::snapshot::capture_ui_snapshot(self),
        };
        if let Ok(mut host) = host.lock() {
            host.draw_frame(frame);
        }
    }

    fn gobj_text(&mut self, target: Option<&Value>, args: &[Value]) -> Result<Value> {
        let numeric = |v: &Value| matches!(v, Value::Matrix(_) | Value::Logical(_));
        let (coordinates, string_index) = match args {
            [x, y, z, _, ..] if numeric(x) && numeric(y) && numeric(z) => (3, 3),
            [x, y, _, ..] if numeric(x) && numeric(y) => (2, 2),
            _ => {
                return Err(
                    "Use text(x, y, str) or text(x, y, z, str) with optional Name,Value pairs"
                        .into(),
                );
            }
        };
        let x = real_vector(&args[0], "text x")?;
        let y = real_vector(&args[1], "text y")?;
        let z = if coordinates == 3 {
            real_vector(&args[2], "text z")?
        } else {
            vec![0.; x.len()]
        };
        if x.is_empty() || x.len() != y.len() || x.len() != z.len() {
            return Err("text coordinates must be nonempty vectors of equal length".into());
        }
        if x.len() > crate::plot::MAX_TEXTS {
            return Err("text creates at most 256 objects per axes".into());
        }
        let strings: Vec<String> = match &args[string_index] {
            Value::Text(s) | Value::QuotedString(s) => vec![s.clone(); x.len()],
            Value::Matrix(m) if m.is_empty() => vec![String::new(); x.len()],
            Value::Matrix(m) => {
                let s = m
                    .data()
                    .iter()
                    .map(|v| format!("{v}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                vec![s; x.len()]
            }
            Value::Container(Array::Cell(_))
            | Value::Container(Array::Str(_))
            | Value::Container(Array::Character(_))
                if x.len() == 1 =>
            {
                vec![super::super::graphics_text_lines(&args[string_index], &self.cancel)?.0]
            }
            Value::Container(Array::Cell(cell)) => {
                if cell.len() != x.len() && cell.len() != 1 {
                    return Err("text requires one string or one cell element per point".into());
                }
                let mut out = Vec::with_capacity(x.len());
                for i in 0..x.len() {
                    let element = cell.get_linear(if cell.len() == 1 { 0 } else { i })?;
                    let value = self.text_arguments(vec![Value::from(element.as_ref().clone())])?;
                    match &value[0] {
                        Value::Text(s) => out.push(s.clone()),
                        _ => return Err("text cell elements must be character text".into()),
                    }
                }
                out
            }
            _ => return Err("text requires character text".into()),
        };
        let properties = &args[string_index + 1..];
        if !properties.len().is_multiple_of(2) || properties.len() > 32 {
            return Err("text properties require up to 16 Name,Value pairs".into());
        }
        let (id, mut figure, axes_id) = self.gobj_target(target)?;
        let figure_id = figure.identity.clone();
        let mut objects = Vec::with_capacity(x.len());
        for i in 0..x.len() {
            let mut object = TextObject::new(x[i], y[i], strings[i].clone());
            object.position[2] = z[i];
            self.apply_text_defaults(&mut object, &figure_id)?;
            object.string = strings[i].clone();
            if x.len() == 1
                && matches!(
                    &args[string_index],
                    Value::QuotedString(_)
                        | Value::Container(Array::Cell(_) | Array::Str(_) | Array::Character(_))
                )
            {
                object.string_lines =
                    super::super::graphics_text_lines(&args[string_index], &self.cancel)?.1;
            }
            for pair in properties.as_chunks::<2>().0 {
                apply_text_object(&mut object, pair, &self.cancel)?;
            }
            object.validate()?;
            objects.push(object);
        }
        let mut created = Vec::with_capacity(objects.len());
        {
            let axes = axes_by_id(&mut figure, &axes_id)?;
            if axes.extras.texts.len() + objects.len() > crate::plot::MAX_TEXTS {
                return Err("Axes exceeds 256 text objects".into());
            }
            for object in objects {
                created.push(Arc::new(GraphicsHandle::axes_object(
                    &figure_id,
                    &axes_id,
                    &object.identity,
                    GraphicsKind::Text,
                )));
                axes.extras.texts.push(object);
            }
        }
        let n = created.len();
        let result =
            Value::Container(GraphicsArray::from_shared(n, 1, created, &self.cancel)?.into());
        self.gobj_publish(id, figure)?;
        Ok(result)
    }

    fn gobj_colormap(&mut self, target: Option<&Value>, args: &[Value]) -> Result<Value> {
        let figure_target =
            target.is_some_and(|t| handle(t).is_ok_and(|h| h.kind() == GraphicsKind::Figure));
        let (id, mut figure, axes_id) = self.gobj_target(target)?;
        let map = match args {
            [] => {
                let axes = axes_by_id(&mut figure, &axes_id)?;
                return map_value(&axes.extras.effective_colormap());
            }
            [Value::Text(name)] if name.eq_ignore_ascii_case("default") => None,
            [Value::Text(name)] => Some(Arc::new(
                colormap::named(name, colormap::DEFAULT_ROWS)?.ok_or_else(|| {
                    format!(
                        "Unknown colormap '{name}'; supported: {}",
                        colormap::NAMES.join(", ")
                    )
                })?,
            )),
            [value] => Some(Arc::new(map_rows(
                value,
                crate::plot::MAX_COLORMAP_ROWS,
                "Colormap",
            )?)),
            _ => return Err("Use colormap(name), colormap(map) or colormap(target, map)".into()),
        };
        if figure_target || target.is_none() {
            // A figure colormap applies to every axes in the figure.
            for axes in figure.all_axes_mut() {
                axes.extras.colormap.clone_from(&map);
            }
        } else {
            axes_by_id(&mut figure, &axes_id)?.extras.colormap = map;
        }
        self.gobj_publish(id, figure)?;
        Ok(Value::Void)
    }

    fn gobj_colorbar(&mut self, target: Option<&Value>, args: &[Value]) -> Result<Value> {
        let (id, mut figure, axes_id) = self.gobj_target(target)?;
        let figure_id = figure.identity.clone();
        let axes = axes_by_id(&mut figure, &axes_id)?;
        let mut location = None;
        let mut rest = args;
        if let [Value::Text(word), tail @ ..] = rest
            && tail.len().is_multiple_of(2)
        {
            match word.to_ascii_lowercase().as_str() {
                "off" | "delete" => {
                    if !tail.is_empty() {
                        return Err("colorbar off takes no further arguments".into());
                    }
                    axes.extras.colorbar = None;
                    self.gobj_publish(id, figure)?;
                    return Ok(Value::Void);
                }
                "hide" => {
                    if let Some(bar) = &mut axes.extras.colorbar {
                        bar.visible = false;
                    }
                    self.gobj_publish(id, figure)?;
                    return Ok(Value::Void);
                }
                "show" => {
                    rest = tail;
                }
                other if other != "location" && other != "visible" && other != "fontsize" => {
                    location = Some(ColorbarLocation::parse(word)?);
                    rest = tail;
                }
                _ => {}
            }
        }
        if !rest.len().is_multiple_of(2) || rest.len() > 16 {
            return Err("colorbar accepts a location and Name,Value pairs".into());
        }
        let mut bar = axes
            .extras
            .colorbar
            .clone()
            .unwrap_or_else(|| Colorbar::new(ColorbarLocation::EastOutside));
        bar.visible = true;
        if let Some(location) = location {
            bar.location = location;
        }
        for pair in rest.as_chunks::<2>().0 {
            match text(&pair[0])?.to_ascii_lowercase().as_str() {
                "location" => bar.location = ColorbarLocation::parse(text(&pair[1])?)?,
                "visible" => bar.visible = on_off_arg(&pair[1], "Visible")?,
                "fontsize" => {
                    let size = real_scalar(&pair[1], "FontSize")?;
                    crate::plot::validate_font_size(size)?;
                    bar.font_size = Some(size);
                }
                other => {
                    return Err(format!(
                        "colorbar supports Location, Visible and FontSize; '{other}' is not supported"
                    ));
                }
            }
        }
        let handle = GraphicsHandle::axes_object(
            &figure_id,
            &axes_id,
            &bar.identity,
            GraphicsKind::Colorbar,
        );
        axes.extras.colorbar = Some(bar);
        let result = Value::Container(GraphicsArray::scalar(handle, &self.cancel)?.into());
        self.gobj_publish(id, figure)?;
        Ok(result)
    }

    fn gobj_clim(&mut self, name: &str, target: Option<&Value>, args: &[Value]) -> Result<Value> {
        let (id, mut figure, axes_id) = self.gobj_target(target)?;
        let axes = axes_by_id(&mut figure, &axes_id)?;
        match args {
            [] => Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                2,
                &axes.color_limits(),
            )?)),
            [Value::Text(mode)] if mode.eq_ignore_ascii_case("auto") => {
                axes.extras.clim = None;
                self.gobj_publish(id, figure)?;
                Ok(Value::Void)
            }
            [Value::Text(mode)] if mode.eq_ignore_ascii_case("manual") => {
                axes.extras.clim = Some(axes.color_limits());
                self.gobj_publish(id, figure)?;
                Ok(Value::Void)
            }
            [value] => {
                axes.extras.clim = Some(limits(value, name)?);
                self.gobj_publish(id, figure)?;
                Ok(Value::Void)
            }
            _ => Err(format!("Use {name}([cmin cmax]), {name}('auto') or {name}")),
        }
    }

    fn gobj_yyaxis(&mut self, target: Option<&Value>, args: &[Value]) -> Result<Value> {
        let [Value::Text(side)] = args else {
            return Err("Use yyaxis left or yyaxis right".into());
        };
        let right = match side.to_ascii_lowercase().as_str() {
            "left" => false,
            "right" => true,
            _ => return Err("yyaxis side must be left or right".into()),
        };
        let (id, mut figure, axes_id) = self.gobj_target(target)?;
        let theme = figure.resolve_appearance(self.graphics_theme_context).theme;
        let axes = axes_by_id(&mut figure, &axes_id)?;
        if axes.yscale == crate::plot::AxisScale::Log {
            return Err(
                "yyaxis supports linear y-axes; log-scale yyaxis is not supported yet".into(),
            );
        }
        let has_explicit_order = axes.extras.color_order.is_some();
        let order = if let Some(cycle) = axes.palette_cycle
            && !has_explicit_order
        {
            Arc::new(
                super::super::graphics_root::automatic_color_order(cycle.version, theme).to_vec(),
            )
        } else {
            axes.extras.effective_color_order()
        };
        let yy = axes.extras.yy.get_or_insert_with(|| YyAxis::new(&order));
        yy.active_right = right;
        self.gobj_publish(id, figure)?;
        Ok(Value::Void)
    }
}

fn subtitle_property(
    subtitle: &Subtitle,
    name: &str,
    default_color: [f64; 3],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Option<Value>> {
    Ok(Some(match name {
        "string" => match &subtitle.string_lines {
            Some(lines) => super::super::graphics_text_value(lines, cancel)?,
            None => string(&subtitle.string),
        },
        "color" => rgb_value(subtitle.color.unwrap_or(default_color))?,
        "fontsize" => scalar(subtitle.font_size_points()),
        "fontweight" => string(font_weight_name(subtitle.bold)),
        "fontangle" => string(font_angle_name(subtitle.italic)),
        "interpreter" => string(subtitle.interpreter.as_str()),
        _ => return Ok(None),
    }))
}

fn font_weight_name(bold: bool) -> &'static str {
    if bold { "bold" } else { "normal" }
}

fn font_angle_name(italic: bool) -> &'static str {
    if italic { "italic" } else { "normal" }
}

pub(in crate::engine) fn text_font_weight(value: &Value) -> Result<bool> {
    match text(value)?.to_ascii_lowercase().as_str() {
        "bold" => Ok(true),
        "normal" => Ok(false),
        _ => Err("FontWeight must be normal or bold".into()),
    }
}

pub(in crate::engine) fn text_font_angle(value: &Value) -> Result<bool> {
    match text(value)?.to_ascii_lowercase().as_str() {
        "italic" => Ok(true),
        "normal" => Ok(false),
        _ => Err("FontAngle must be normal or italic".into()),
    }
}

pub(super) fn apply_subtitle_property(
    subtitle: &mut Subtitle,
    pair: &[Value; 2],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    match text(&pair[0])?.to_ascii_lowercase().as_str() {
        "string" => {
            (subtitle.string, subtitle.string_lines) =
                super::super::graphics_text_lines(&pair[1], cancel)?;
        }
        "color" => subtitle.color = Some(any_rgb(&pair[1], cancel)?),
        "fontsize" => subtitle.font_size = Some(real_scalar(&pair[1], "FontSize")?),
        "fontweight" => subtitle.bold = text_font_weight(&pair[1])?,
        "fontangle" => subtitle.italic = text_font_angle(&pair[1])?,
        "interpreter" => {
            subtitle.interpreter = crate::plot::text_markup::Interpreter::parse(text(&pair[1])?)?;
        }
        other => return Err(format!("Unsupported Subtitle property '{other}'")),
    }
    Ok(())
}

fn apply_text_object(
    object: &mut TextObject,
    pair: &[Value; 2],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    match text(&pair[0])?.to_ascii_lowercase().as_str() {
        "string" => {
            let (string, lines) = super::super::graphics_text_lines(&pair[1], cancel)?;
            object.string = string;
            object.string_lines = lines;
        }
        "position" => {
            let v = real_vector(&pair[1], "Position")?;
            match v.as_slice() {
                [x, y] => object.position = [*x, *y, 0.],
                [x, y, z] => object.position = [*x, *y, *z],
                _ => return Err("Text Position requires [x y] or [x y z]".into()),
            }
        }
        "color" => object.color = Some(any_rgb(&pair[1], cancel)?),
        "fontsize" => {
            object.font_size = real_scalar(&pair[1], "FontSize")?;
            object.font_size_explicit = true;
        }
        "fontweight" => object.bold = text_font_weight(&pair[1])?,
        "fontangle" => object.italic = text_font_angle(&pair[1])?,
        "horizontalalignment" => {
            object.horizontal = crate::plot::parse_horizontal(text(&pair[1])?)?
        }
        "verticalalignment" => {
            object.vertical = crate::plot::VerticalAlign::parse(text(&pair[1])?)?
        }
        "rotation" => object.rotation = real_scalar(&pair[1], "Rotation")?,
        "visible" => object.visible = on_off_arg(&pair[1], "Visible")?,
        "units" if text(&pair[1])?.eq_ignore_ascii_case("data") => {}
        "interpreter" => {
            object.interpreter = crate::plot::text_markup::Interpreter::parse(text(&pair[1])?)?
        }
        other => {
            return Err(format!(
                "Text supports String, Position, Color, FontSize, FontWeight, HorizontalAlignment, VerticalAlignment, Rotation, Visible, data Units and Interpreter none/tex; '{other}' is not supported"
            ));
        }
    }
    Ok(())
}

fn label_style_mut(axes: &mut Axes, target: TextTarget) -> &mut LabelStyle {
    match target {
        TextTarget::Title => &mut axes.extras.title_style,
        TextTarget::XLabel => &mut axes.extras.label_styles[0],
        TextTarget::YLabel => &mut axes.extras.label_styles[1],
        TextTarget::ZLabel => &mut axes.extras.label_styles[2],
        TextTarget::RightYLabel => &mut axes.extras.yy.as_mut().unwrap().right_label_style,
        _ => unreachable!("label_style_mut called for non-label target"),
    }
}

fn set_label_font_style(style: &mut LabelStyle, name: &str, value: &Value) -> Result<()> {
    match name {
        "fontsize" => {
            let size = real_scalar(value, "FontSize")?;
            crate::plot::validate_font_size(size)?;
            style.font_size = Some(size);
        }
        "fontweight" => style.bold = Some(text_font_weight(value)?),
        "fontangle" => style.italic = Some(text_font_angle(value)?),
        _ => return Err(format!("Unsupported label font property '{name}'")),
    }
    Ok(())
}

/// `set` pairs handled by this module, applied to the candidate figure.
/// Returns the pairs left for the original property code.
pub(in crate::engine) fn set_extended(
    engine: &Engine,
    h: &GraphicsHandle,
    figure: &mut Figure,
    args: &[Value],
) -> Result<Vec<Value>> {
    let cancel = &engine.cancel;
    let mut remaining = Vec::new();
    match h.kind() {
        GraphicsKind::Figure => {
            for pair in args.as_chunks::<2>().0 {
                if text(&pair[0])?.eq_ignore_ascii_case("colormap") {
                    let map = Arc::new(map_rows(
                        &pair[1],
                        crate::plot::MAX_COLORMAP_ROWS,
                        "Colormap",
                    )?);
                    for axes in figure.all_axes_mut() {
                        axes.extras.colormap = Some(map.clone());
                    }
                } else {
                    remaining.extend_from_slice(pair);
                }
            }
        }
        GraphicsKind::Image => {
            let axes_id = h.axes.clone().unwrap();
            let axes = axes_by_id(figure, &axes_id)?;
            let image = axes
                .extras
                .images
                .iter_mut()
                .find(|i| i.identity.same_object(&h.object))
                .ok_or("Image is no longer live")?;
            for pair in args.as_chunks::<2>().0 {
                super::image_commands::image_set(engine, image, pair)?;
            }
        }
        GraphicsKind::ConstantLine => {
            let axes_id = h.axes.clone().unwrap();
            let axes = axes_by_id(figure, &axes_id)?;
            let line = axes
                .extras
                .constant_lines
                .iter_mut()
                .find(|l| l.identity.same_object(&h.object))
                .ok_or("ConstantLine is no longer live")?;
            for pair in args.as_chunks::<2>().0 {
                super::shapes_commands::constant_line_set(line, pair)?;
            }
        }
        GraphicsKind::Rectangle => {
            let axes_id = h.axes.clone().unwrap();
            let axes = axes_by_id(figure, &axes_id)?;
            let rect = axes
                .extras
                .rectangles
                .iter_mut()
                .find(|r| r.identity.same_object(&h.object))
                .ok_or("Rectangle is no longer live")?;
            for pair in args.as_chunks::<2>().0 {
                super::shapes_commands::rectangle_set(rect, pair)?;
            }
        }
        GraphicsKind::TiledLayout => {
            let layout = figure
                .layout
                .as_mut()
                .filter(|l| l.tiled.is_some() && l.identity.same_object(&h.object))
                .ok_or("TiledChartLayout is no longer live")?;
            for pair in args.as_chunks::<2>().0 {
                match text(&pair[0])?.to_ascii_lowercase().as_str() {
                    "tilespacing" => {
                        layout.tile_spacing = crate::plot::TileSpacing::parse(text(&pair[1])?)?;
                    }
                    "padding" => {
                        layout.padding = crate::plot::TilePadding::parse(text(&pair[1])?)?;
                    }
                    other => {
                        return Err(format!(
                            "Setting TiledChartLayout property '{other}' is not supported"
                        ));
                    }
                }
            }
        }
        GraphicsKind::Text if h.axes.is_none() => {
            let layout = figure
                .layout
                .as_mut()
                .ok_or("Text object is no longer live")?;
            if h.object.same_object(&layout.subtitle_identity) {
                for pair in args.as_chunks::<2>().0 {
                    apply_subtitle_property(&mut layout.subtitle, pair, cancel)?;
                }
                return Ok(remaining);
            }
            let index = if h.object.same_object(&layout.title_identity) {
                0
            } else if h.object.same_object(&layout.xlabel_identity) {
                1
            } else if h.object.same_object(&layout.ylabel_identity) {
                2
            } else {
                return Err("Text object is no longer live".into());
            };
            for pair in args.as_chunks::<2>().0 {
                match text(&pair[0])?.to_ascii_lowercase().as_str() {
                    "string" => {
                        let (value, lines) = super::super::graphics_text_lines(&pair[1], cancel)?;
                        layout.text_lines[index] = lines;
                        if h.object.same_object(&layout.title_identity) {
                            layout.title = value;
                        } else if h.object.same_object(&layout.xlabel_identity) {
                            layout.xlabel = value;
                        } else if h.object.same_object(&layout.ylabel_identity) {
                            layout.ylabel = value;
                        } else {
                            return Err("Text object is no longer live".into());
                        }
                    }
                    "interpreter" => {
                        layout.text_interpreters[index] =
                            crate::plot::text_markup::Interpreter::parse(text(&pair[1])?)?
                    }
                    "fontsize" => {
                        let size = real_scalar(&pair[1], "FontSize")?;
                        crate::plot::validate_font_size(size)?;
                        if index == 0 {
                            layout.title_style.font_size = Some(size);
                        } else {
                            layout.label_styles[index - 1].font_size = Some(size);
                        }
                    }
                    "fontweight" => {
                        let bold = text_font_weight(&pair[1])?;
                        if index == 0 {
                            layout.title_style.bold = Some(bold);
                        } else {
                            layout.label_styles[index - 1].bold = Some(bold);
                        }
                    }
                    "fontangle" => {
                        let italic = text_font_angle(&pair[1])?;
                        if index == 0 {
                            layout.title_style.italic = Some(italic);
                        } else {
                            layout.label_styles[index - 1].italic = Some(italic);
                        }
                    }
                    other => {
                        return Err(format!(
                            "Setting layout Text property '{other}' is not supported"
                        ));
                    }
                }
            }
        }
        GraphicsKind::Text => {
            let axes_id = h.axes.clone().unwrap();
            let axes = axes_by_id(figure, &axes_id)?;
            let target = text_target(axes, &h.object).ok_or("Text object is no longer live")?;
            for pair in args.as_chunks::<2>().0 {
                let name = text(&pair[0])?.to_ascii_lowercase();
                match target {
                    TextTarget::Subtitle => {
                        apply_subtitle_property(&mut axes.extras.subtitle, pair, cancel)?
                    }
                    TextTarget::Object(i) => {
                        apply_text_object(&mut axes.extras.texts[i], pair, cancel)?;
                        axes.extras.texts[i].validate()?;
                    }
                    _ => match name.as_str() {
                        "string" => {
                            let (value, lines) =
                                super::super::graphics_text_lines(&pair[1], cancel)?;
                            let index = match target {
                                TextTarget::Title => Some(0),
                                TextTarget::XLabel => Some(1),
                                TextTarget::YLabel => Some(2),
                                TextTarget::ZLabel => Some(3),
                                _ => None,
                            };
                            if let Some(i) = index {
                                axes.extras.label_lines[i] = lines;
                            }
                            match target {
                                TextTarget::Title => axes.title = value,
                                TextTarget::XLabel => axes.xlabel = value,
                                TextTarget::YLabel => axes.ylabel = value,
                                TextTarget::ZLabel => axes.zlabel = value,
                                TextTarget::RightYLabel => {
                                    axes.extras.yy.as_mut().unwrap().right_label = value
                                }
                                TextTarget::ColorbarLabel => {
                                    axes.extras.colorbar.as_mut().unwrap().label = value
                                }
                                TextTarget::ColorbarTitle => {
                                    axes.extras.colorbar.as_mut().unwrap().title = value
                                }
                                TextTarget::Subtitle | TextTarget::Object(_) => unreachable!(),
                            }
                        }
                        "color" => {
                            let rgb = any_rgb(&pair[1], cancel)?;
                            let colors = axes.colors.get_or_insert_with(Default::default);
                            match target {
                                TextTarget::Title => colors.title = Some(rgb),
                                TextTarget::XLabel => colors.xlabel = Some(rgb),
                                TextTarget::YLabel => colors.ylabel = Some(rgb),
                                TextTarget::ZLabel => {
                                    return Err("ZLabel Color is not supported yet".into());
                                }
                                TextTarget::RightYLabel => {
                                    let yy = axes.extras.yy.as_mut().unwrap();
                                    yy.right_color = rgb;
                                    yy.right_explicit = true;
                                }
                                TextTarget::ColorbarLabel => {
                                    return Err("Colorbar label Color is not supported yet".into());
                                }
                                TextTarget::ColorbarTitle => {
                                    return Err("Colorbar title Color is not supported yet".into());
                                }
                                TextTarget::Subtitle | TextTarget::Object(_) => unreachable!(),
                            }
                        }
                        "fontsize" | "fontweight" | "fontangle"
                            if !matches!(
                                target,
                                TextTarget::ColorbarLabel | TextTarget::ColorbarTitle
                            ) =>
                        {
                            set_label_font_style(label_style_mut(axes, target), &name, &pair[1])?;
                        }
                        "interpreter" => {
                            let interpreter =
                                crate::plot::text_markup::Interpreter::parse(text(&pair[1])?)?;
                            match target {
                                TextTarget::Title => {
                                    axes.extras.label_interpreters[0] = interpreter
                                }
                                TextTarget::XLabel => {
                                    axes.extras.label_interpreters[1] = interpreter
                                }
                                TextTarget::YLabel => {
                                    axes.extras.label_interpreters[2] = interpreter
                                }
                                TextTarget::ZLabel => {
                                    axes.extras.label_interpreters[3] = interpreter
                                }
                                TextTarget::Subtitle
                                | TextTarget::RightYLabel
                                | TextTarget::ColorbarLabel
                                | TextTarget::ColorbarTitle
                                | TextTarget::Object(_) => {}
                            }
                        }
                        "visible" if on_off_arg(&pair[1], "Visible")? => {}
                        _ => {
                            return Err(format!(
                                "Titles and labels support String, Color and (title only) FontSize and FontWeight; '{}' is not supported yet",
                                text(&pair[0])?
                            ));
                        }
                    },
                }
            }
        }
        GraphicsKind::Colorbar => {
            let axes_id = h.axes.clone().unwrap();
            let axes = axes_by_id(figure, &axes_id)?;
            for pair in args.as_chunks::<2>().0 {
                let name = text(&pair[0])?.to_ascii_lowercase();
                if name == "limits" {
                    axes.extras.clim = Some(limits(&pair[1], "Limits")?);
                    continue;
                }
                let bar = axes
                    .extras
                    .colorbar
                    .as_mut()
                    .ok_or("Colorbar is no longer live")?;
                match name.as_str() {
                    "location" => bar.location = ColorbarLocation::parse(text(&pair[1])?)?,
                    "visible" => bar.visible = on_off_arg(&pair[1], "Visible")?,
                    "fontsize" => {
                        let size = real_scalar(&pair[1], "FontSize")?;
                        crate::plot::validate_font_size(size)?;
                        bar.font_size = Some(size);
                    }
                    _ => {
                        return Err(format!(
                            "Supported colorbar properties: Limits, Location, Visible and FontSize; '{}' is not supported yet",
                            text(&pair[0])?
                        ));
                    }
                }
            }
        }
        GraphicsKind::NumericRuler => {
            let axes_id = h.axes.clone().unwrap();
            let axes = axes_by_id(figure, &axes_id)?;
            let axis = axes
                .extras
                .ruler_index(&h.object)
                .ok_or("NumericRuler is no longer live")?;
            let mut sync_x = false;
            let mut sync_y = false;
            for pair in args.as_chunks::<2>().0 {
                let name = text(&pair[0])?.to_ascii_lowercase();
                if matches!(name.as_str(), "limits" | "limitsmode") {
                    if axis == 0 {
                        sync_x = true;
                    } else if axis == 1 {
                        sync_y = true;
                    }
                }
                super::super::graphics_ticks::ruler_set_property(
                    axes,
                    axis,
                    &name,
                    &pair[1],
                    &engine.cancel,
                )?;
            }
            if sync_x || sync_y {
                figure.sync_linked_axes(&axes_id, sync_x, sync_y);
            }
        }
        GraphicsKind::Axes => {
            let axes_id = h.axes.clone().unwrap();
            let axes = axes_by_id(figure, &axes_id)?;
            for pair in args.as_chunks::<2>().0 {
                let name = text(&pair[0])?.to_ascii_lowercase();
                let yy_active = axes.extras.yy.as_ref().map(|y| y.active_right);
                match name.as_str() {
                    "tag" => {
                        let tag = text(&pair[1])?;
                        if tag.len() > 4096 {
                            return Err("Axes Tag exceeds 4096 bytes".into());
                        }
                        axes.extras.tag = tag.into();
                    }
                    "visible" => axes.extras.axis_off = !on_off_arg(&pair[1], "Visible")?,
                    "ticklabelinterpreter" => {
                        axes.extras.tick_label_interpreter =
                            crate::plot::text_markup::Interpreter::parse(text(&pair[1])?)?
                    }
                    "fontsize" => {
                        let size = real_scalar(&pair[1], "FontSize")?;
                        crate::plot::validate_font_size(size)?;
                        axes.extras.font_size = Some(size);
                    }
                    "view" => {
                        let view = real_vector(&pair[1], "View")?;
                        let [az, el] = view.as_slice() else {
                            return Err("View requires [azimuth elevation]".into());
                        };
                        axes.view = [*az, *el];
                        crate::plot::view3d::validate_view(axes.view)?;
                    }
                    "linewidth" => {
                        let w = real_scalar(&pair[1], "LineWidth")?;
                        crate::plot::validate_line_width(w)?;
                        axes.extras.line_width = Some(w);
                    }
                    "tickdir" => {
                        axes.extras.tick_dir = Some(crate::plot::TickDir::parse(text(&pair[1])?)?);
                    }
                    "layer" => {
                        axes.extras.layer = crate::plot::AxesLayer::parse(text(&pair[1])?)?;
                    }
                    "ticklength" => {
                        let v = real_vector(&pair[1], "TickLength")?;
                        let [a, b] = v.as_slice() else {
                            return Err("TickLength requires a 2-element vector".into());
                        };
                        crate::plot::validate_tick_length([*a, *b])?;
                        axes.extras.tick_length = Some([*a, *b]);
                    }
                    "xtick" | "ytick" | "ztick" => {
                        let axis = match name.as_str() {
                            "xtick" => 0,
                            "ytick" => 1,
                            _ => 2,
                        };
                        let ticks = super::super::plot_time_axes::parse_ruler_values(
                            &axes.extras.rulers[axis].kind,
                            &pair[1],
                            "Ticks",
                            &engine.cancel,
                        )?;
                        crate::plot::validate_ticks(&ticks)?;
                        axes.extras.rulers[axis].ticks = Some(ticks);
                    }
                    "xtickmode" | "ytickmode" | "ztickmode" => {
                        let axis = match name.as_str() {
                            "xtickmode" => 0,
                            "ytickmode" => 1,
                            _ => 2,
                        };
                        match text(&pair[1])?.to_ascii_lowercase().as_str() {
                            "auto" => axes.extras.rulers[axis].ticks = None,
                            "manual" => {
                                let (ticks, _) = axes.effective_ticks(axis);
                                axes.extras.rulers[axis].ticks = Some(ticks);
                            }
                            _ => return Err("TickMode must be auto or manual".into()),
                        }
                    }
                    "xticklabel" | "yticklabel" | "zticklabel" => {
                        let axis = match name.as_str() {
                            "xticklabel" => 0,
                            "yticklabel" => 1,
                            _ => 2,
                        };
                        axes.extras.rulers[axis].tick_labels =
                            Some(super::super::graphics_ticks::parse_tick_labels(
                                &pair[1],
                                &engine.cancel,
                            )?);
                    }
                    "xticklabelmode" | "yticklabelmode" | "zticklabelmode" => {
                        let axis = match name.as_str() {
                            "xticklabelmode" => 0,
                            "yticklabelmode" => 1,
                            _ => 2,
                        };
                        match text(&pair[1])?.to_ascii_lowercase().as_str() {
                            "auto" => axes.extras.rulers[axis].tick_labels = None,
                            "manual" => {
                                let labels = axes.effective_tick_labels(axis);
                                axes.extras.rulers[axis].tick_labels = Some(labels);
                            }
                            _ => return Err("TickLabelMode must be auto or manual".into()),
                        }
                    }
                    "xticklabelrotation" | "yticklabelrotation" | "zticklabelrotation" => {
                        let axis = match name.as_str() {
                            "xticklabelrotation" => 0,
                            "yticklabelrotation" => 1,
                            _ => 2,
                        };
                        let deg = real_scalar(&pair[1], "TickLabelRotation")?;
                        axes.extras.rulers[axis].tick_label_rotation = deg;
                    }
                    "zlim" => {
                        let lim = {
                            let v = real_vector(&pair[1], "ZLim")?;
                            let [a, b] = v.as_slice() else {
                                return Err("ZLim requires two elements".into());
                            };
                            crate::plot::validate_axis_limits([*a, *b])?;
                            [*a, *b]
                        };
                        axes.zlim = if lim == [f64::NEG_INFINITY, f64::INFINITY] {
                            None
                        } else {
                            Some(lim)
                        };
                    }
                    "zlimmode" => match text(&pair[1])?.to_ascii_lowercase().as_str() {
                        "auto" => axes.zlim = None,
                        "manual" => axes.zlim = Some(axes.zbounds()),
                        _ => return Err("ZLimMode must be auto or manual".into()),
                    },
                    "zgrid" => axes.zgrid = on_off_arg(&pair[1], "ZGrid")?,
                    "box" => axes.box_outline = on_off_arg(&pair[1], "Box")?,
                    "xminorgrid" => axes.minor_grid[0] = on_off_arg(&pair[1], "XMinorGrid")?,
                    "yminorgrid" => axes.minor_grid[1] = on_off_arg(&pair[1], "YMinorGrid")?,
                    "zminorgrid" => axes.minor_grid[2] = on_off_arg(&pair[1], "ZMinorGrid")?,
                    "colororder" => {
                        let rows =
                            map_rows(&pair[1], crate::plot::MAX_COLOR_ORDER_ROWS, "ColorOrder")?;
                        apply_color_order_rows(axes, rows);
                    }
                    "colormap" => {
                        axes.extras.colormap = Some(Arc::new(map_rows(
                            &pair[1],
                            crate::plot::MAX_COLORMAP_ROWS,
                            "Colormap",
                        )?));
                    }
                    "clim" => axes.extras.clim = Some(limits(&pair[1], "CLim")?),
                    "climmode" => match text(&pair[1])?.to_ascii_lowercase().as_str() {
                        "auto" => axes.extras.clim = None,
                        "manual" => axes.extras.clim = Some(axes.color_limits()),
                        _ => return Err("CLimMode must be auto or manual".into()),
                    },
                    "ylim" if yy_active == Some(true) => {
                        axes.extras.yy.as_mut().unwrap().right_ylim =
                            Some(limits(&pair[1], "YLim")?);
                    }
                    "ycolor" if yy_active.is_some() => {
                        let rgb = any_rgb(&pair[1], cancel)?;
                        let yy = axes.extras.yy.as_mut().unwrap();
                        if yy.active_right {
                            yy.right_color = rgb;
                            yy.right_explicit = true;
                        } else {
                            yy.left_color = rgb;
                            yy.left_explicit = true;
                        }
                    }
                    "yaxislocation" if yy_active.is_some() => {
                        let side = text(&pair[1])?.to_ascii_lowercase();
                        axes.extras.yy.as_mut().unwrap().active_right = match side.as_str() {
                            "left" => false,
                            "right" => true,
                            _ => return Err("YAxisLocation must be left or right".into()),
                        };
                    }
                    "xcolor"
                    | "color"
                    | "xgrid"
                    | "ygrid"
                    | "nextplot"
                    | "xscale"
                    | "yscale"
                    | "xlim"
                    | "ylim"
                    | "units"
                    | "position"
                    | "innerposition"
                    | "outerposition"
                    | "positionconstraint"
                    | "activepositionproperty"
                    | "ycolor"
                    | "gridcolor"
                    | "gridalpha"
                    | "xcolormode"
                    | "ycolormode"
                    | "gridcolormode"
                    | "gridalphamode" => remaining.extend_from_slice(pair),
                    _ => {
                        return Err(format!(
                            "Unsupported axes property '{}'; no property transaction was published",
                            text(&pair[0])?
                        ));
                    }
                }
            }
        }
        GraphicsKind::Series => {
            let axes_id = h.axes.clone().unwrap();
            let axes = axes_by_id(figure, &axes_id)?;
            let index = axes
                .series
                .iter()
                .position(|s| s.identity.same_object(&h.object))
                .ok_or("Series is no longer live")?;
            let mut xdata = None;
            let mut ydata = None;
            let mut zdata = None;
            for pair in args.as_chunks::<2>().0 {
                match text(&pair[0])?.to_ascii_lowercase().as_str() {
                    "xdata" => {
                        xdata = Some(parse_series_data(
                            &axes.extras.rulers[0].kind,
                            &pair[1],
                            "XData",
                            cancel,
                        )?)
                    }
                    "ydata" => {
                        ydata = Some(parse_series_data(
                            &axes.extras.rulers[1].kind,
                            &pair[1],
                            "YData",
                            cancel,
                        )?)
                    }
                    "zdata" => zdata = Some(real_vector(&pair[1], "ZData")?),
                    _ => remaining.extend_from_slice(pair),
                }
            }
            if xdata.is_some() || ydata.is_some() {
                replace_data(&mut axes.series[index], xdata, ydata, cancel)?;
            }
            if let Some(z) = zdata {
                let series = &mut axes.series[index];
                if !matches!(
                    series.kind,
                    crate::plot::PlotKind::Line | crate::plot::PlotKind::Scatter
                ) {
                    return Err("ZData belongs only to line and scatter objects".into());
                }
                series.z = z.into();
                if series.z.is_empty() {
                    // Empty ZData makes the line 2-D again.
                } else if series.z.len() != series.points.len() {
                    return Err("ZData must have one value per XData/YData point; set them together with set(h, 'XData', x, 'YData', y, 'ZData', z)".into());
                }
            }
        }
        _ => remaining.extend_from_slice(args),
    }
    Ok(remaining)
}

fn parse_series_data(
    kind: &crate::plot::RulerKind,
    value: &Value,
    what: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Vec<f64>> {
    match kind {
        crate::plot::RulerKind::Numeric => real_vector(value, what),
        _ => super::super::plot_time_axes::parse_ruler_values(kind, value, what, cancel),
    }
}

/// New XData/YData for one series. Both vectors keep one length; an implicit
/// 1..n XData follows a YData of a new length.
fn replace_data(
    series: &mut Series,
    xdata: Option<Vec<f64>>,
    ydata: Option<Vec<f64>>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    if series.kind == crate::plot::PlotKind::Patch {
        return Err(
            "Patch coordinates are edited through Vertices, which is not supported yet".into(),
        );
    }
    let old: Vec<[f64; 2]> = series.points.iter().collect();
    crate::dsp::check_cancel(cancel)?;
    let implicit = old.iter().enumerate().all(|(i, p)| p[0] == (i + 1) as f64);
    let x = match (&xdata, &ydata) {
        (Some(x), _) => x.clone(),
        (None, Some(y)) if y.len() != old.len() && implicit => {
            (1..=y.len()).map(|i| i as f64).collect()
        }
        _ => old.iter().map(|p| p[0]).collect(),
    };
    let y = ydata.unwrap_or_else(|| old.iter().map(|p| p[1]).collect());
    if x.len() != y.len() {
        return Err("XData and YData must have the same length; set both with set(h, 'XData', x, 'YData', y)".into());
    }
    if x.len() > crate::plot::MAX_POINTS {
        return Err("Series data exceeds 1000000 points".into());
    }
    series.points = x
        .into_iter()
        .zip(y)
        .map(|(a, b)| [a, b])
        .collect::<Vec<_>>()
        .into();
    Ok(())
}

/// Called by `plot` and friends after they built their candidate figure:
/// keeps axes-owned objects that survive a replacing plot (title/label
/// identities, colormap, colorbar), and routes new series to the active
/// yyaxis side with that side's colour.
pub(in crate::engine) fn plot_carry_over(
    previous: Option<&Figure>,
    figure: &mut Figure,
    series: &mut [Series],
) -> Result<()> {
    let held = previous.is_some_and(|f| f.hold);
    let previous_axes = previous
        .map(|f| &f.axes)
        .filter(|a| a.identity.same_object(&figure.axes.identity));
    if let Some(old) = previous_axes
        && !held
    {
        let extras = &mut figure.axes.extras;
        extras.title_identity = old.extras.title_identity.clone();
        extras.subtitle_identity = old.extras.subtitle_identity.clone();
        extras.xlabel_identity = old.extras.xlabel_identity.clone();
        extras.ylabel_identity = old.extras.ylabel_identity.clone();
        extras.zlabel_identity = old.extras.zlabel_identity.clone();
        extras.ruler_identities = old.extras.ruler_identities.clone();
        extras.colormap.clone_from(&old.extras.colormap);
        extras.colorbar.clone_from(&old.extras.colorbar);
        extras.color_order.clone_from(&old.extras.color_order);
        extras.tag.clone_from(&old.extras.tag);
        if let Some(yy) = &old.extras.yy {
            // A replacing plot on one yyaxis side keeps the other side, the
            // labels and limits of the axes.
            let mut kept = old.clone();
            kept.series
                .retain(|s| yy.is_right(&s.identity) != yy.active_right);
            let yy = kept.extras.yy.as_mut().unwrap();
            let survivors: Vec<_> = kept.series.iter().map(|s| s.identity.clone()).collect();
            yy.right
                .retain(|id| survivors.iter().any(|s| s.same_object(id)));
            if yy.active_right {
                yy.right_ylim = None;
            } else {
                kept.ylim = None;
            }
            kept.xscale = figure.axes.xscale;
            // The replacing plot chose the view: view(3) for plot3/scatter3,
            // view(2) for a 2-D plot.
            kept.view = figure.axes.view;
            kept.palette_cycle = figure.axes.palette_cycle;
            kept.legend_state = None;
            kept.extras.legend_style = Default::default();
            kept.baselines.clear();
            figure.axes = kept;
        }
    }
    let axes = &mut figure.axes;
    if let Some(yy) = &mut axes.extras.yy {
        let (color, side_explicit) = if yy.active_right {
            (yy.right_color, yy.right_explicit)
        } else {
            (yy.left_color, yy.left_explicit)
        };
        for s in series.iter_mut() {
            if !s.explicit_color {
                s.color = color.map(|v| (v * 255.).round() as u8);
                if !side_explicit
                    && axes.extras.color_order.is_none()
                    && let Some(cycle) = axes.palette_cycle
                {
                    s.auto_palette = Some(crate::plot::AutoPalette {
                        version: cycle.version,
                        slot: if yy.active_right { 1 } else { 0 },
                    });
                } else if side_explicit {
                    s.explicit_color = true;
                }
            }
            if yy.active_right {
                yy.right.push(s.identity.clone());
            }
        }
        if yy.right.len() > crate::plot::MAX_PLOT_SERIES {
            return Err("Right y-axis exceeds series storage budget".into());
        }
    }
    Ok(())
}

/// Called by `plot` and friends right after `Figure::add`: automatic-colour
/// series added from index `first` on take their colour from an explicit
/// ColorOrder and stay automatic (a later ColorOrder recolours them). Runs
/// after `add` because `add` gives every automatic series a palette colour.
pub(in crate::engine) fn plot_apply_color_order(figure: &mut Figure, first: usize) {
    let axes = &mut figure.axes;
    let Some(order) = &axes.extras.color_order else {
        return;
    };
    for (i, s) in axes.series.iter_mut().enumerate().skip(first) {
        if !s.explicit_color {
            let index = axes
                .extras
                .yy
                .as_ref()
                .map_or(i, |yy| usize::from(yy.is_right(&s.identity)));
            s.color = order_color(order, index);
            s.auto_palette = None;
        }
    }
}
pub(in crate::engine) fn apply_color_order_rows(axes: &mut Axes, rows: Vec<[f64; 3]>) {
    // Existing automatic-colour series follow the new order
    // and stay automatic, so every later ColorOrder
    // recolours them; explicit colours stay untouched.
    let yy = &mut axes.extras.yy;
    if let Some(yy) = yy.as_mut() {
        if !yy.left_explicit {
            yy.left_color = rows[0];
        }
        if !yy.right_explicit {
            yy.right_color = rows[1 % rows.len()];
        }
    }
    for (i, s) in axes.series.iter_mut().enumerate() {
        if !s.explicit_color {
            let index = yy
                .as_ref()
                .map_or(i, |yy| usize::from(yy.is_right(&s.identity)));
            s.color = order_color(&rows, index);
            s.auto_palette = None;
        }
    }
    axes.extras.color_order = Some(Arc::new(rows));
}
fn order_color(order: &[[f64; 3]], index: usize) -> [u8; 3] {
    order[index % order.len()].map(|v| (v * 255.).round() as u8)
}
