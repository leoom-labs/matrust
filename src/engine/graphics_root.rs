//! Root graphics object (`groot` / handle `0`) and session-level default
//! property values (`Default<Type><Prop>`, `Factory<Type><Prop>`, `'remove'`).
use super::{Engine, Value};
use crate::plot::{
    AxesGridStyle, ColorMode, Figure, ImageMapping, ImageObject, LinePattern, MarkerShape,
    PlotKind, Series, TextObject, VerticalAlign,
};
use crate::{Array, GraphicsArray, GraphicsHandle, Matrix, Result, StructArray};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const SCREEN_SIZE: [f64; 4] = [1.0, 1.0, 1920.0, 1080.0];
const SCREEN_PPI: f64 = 96.0;

pub(in crate::engine) fn automatic_color_order(
    version: crate::plot::PaletteVersion,
    theme: crate::plot::BaseTheme,
) -> [[f64; 3]; crate::plot::COLORS.len()] {
    std::array::from_fn(|slot| {
        version
            .color(slot as u8, theme)
            .map(|v| f64::from(v) / 255.)
    })
}

pub(in crate::engine) fn retheme_yyaxis(
    figure: &mut crate::plot::Figure,
    host: crate::plot::BaseTheme,
) {
    let theme = figure.resolve_appearance(host).theme;
    for ax in figure.all_axes_mut() {
        let Some(cycle) = ax.palette_cycle else {
            continue;
        };
        if ax.extras.color_order.is_some() {
            continue;
        }
        let Some(yy) = &mut ax.extras.yy else {
            continue;
        };
        let order = automatic_color_order(cycle.version, theme);
        if !yy.left_explicit {
            yy.left_color = order[0];
        }
        if !yy.right_explicit {
            yy.right_color = order[1 % order.len()];
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::engine) enum DefaultObjectType {
    Figure,
    Axes,
    Line,
    Text,
    Patch,
    Surface,
    Image,
}

impl DefaultObjectType {
    pub(in crate::engine) fn as_str(self) -> &'static str {
        match self {
            Self::Figure => "Figure",
            Self::Axes => "Axes",
            Self::Line => "Line",
            Self::Text => "Text",
            Self::Patch => "Patch",
            Self::Surface => "Surface",
            Self::Image => "Image",
        }
    }

    fn from_prefix(rest: &str) -> Option<(Self, &str)> {
        let lower = rest.to_ascii_lowercase();
        for (prefix, kind) in [
            ("figure", Self::Figure),
            ("axes", Self::Axes),
            ("line", Self::Line),
            ("text", Self::Text),
            ("patch", Self::Patch),
            ("surface", Self::Surface),
            ("image", Self::Image),
        ] {
            if lower.starts_with(prefix) {
                return Some((kind, &rest[prefix.len()..]));
            }
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine) enum DefaultQueryKind {
    Default,
    Factory,
}

#[derive(Clone, Debug)]
pub(in crate::engine) struct DefaultProperty {
    pub value: Value,
    is_factory: bool,
}

impl DefaultProperty {
    pub fn parse(
        object: DefaultObjectType,
        property: &'static str,
        value: &Value,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        let is_factory = matches!(value, Value::Text(s) if s.eq_ignore_ascii_case("factory"));
        let value = if is_factory {
            factory_value(object, property)?
        } else {
            validate_default_value(object, property, value, cancel)?;
            value.clone()
        };
        Ok(Self { value, is_factory })
    }
}

pub(in crate::engine) type GraphicsDefaults =
    BTreeMap<(DefaultObjectType, &'static str), DefaultProperty>;

#[derive(Clone, Debug, Default)]
pub(in crate::engine) struct RootState {
    pub show_hidden_handles: bool,
    pub tag: String,
    pub user_data: Option<Value>,
    pub defaults: GraphicsDefaults,
}

pub(in crate::engine) type AxesDefaultValues = Vec<(&'static str, Value)>;

#[derive(Clone)]
pub(in crate::engine) struct AxesCreationDefaults {
    values: AxesDefaultValues,
    pending_figure_axes: bool,
}

const FIGURE_PROPS: &[&str] = &[
    "Name",
    "NumberTitle",
    "Color",
    "Theme",
    "ThemeMode",
    "ColorMode",
    "InvertHardcopy",
    "Colormap",
    "Visible",
    "Position",
    "InnerPosition",
    "OuterPosition",
    "Units",
    "Resize",
    "WindowState",
];

const AXES_PROPS: &[&str] = &[
    "FontSize",
    "Color",
    "XColor",
    "YColor",
    "GridColor",
    "GridAlpha",
    "XColorMode",
    "YColorMode",
    "GridColorMode",
    "GridAlphaMode",
    "XGrid",
    "YGrid",
    "ZGrid",
    "XMinorGrid",
    "YMinorGrid",
    "ZMinorGrid",
    "Box",
    "XScale",
    "YScale",
    "NextPlot",
    "ColorOrder",
    "Colormap",
    "View",
    "Units",
];

const LINE_PROPS: &[&str] = &[
    "LineWidth",
    "Color",
    "LineStyle",
    "Marker",
    "MarkerSize",
    "MarkerFaceColor",
    "DisplayName",
    "Visible",
];

const TEXT_PROPS: &[&str] = &[
    "String",
    "Color",
    "FontSize",
    "FontWeight",
    "HorizontalAlignment",
    "VerticalAlignment",
    "Rotation",
    "Visible",
    "Units",
    "Interpreter",
];

const PATCH_PROPS: &[&str] = &[
    "FaceColor",
    "EdgeColor",
    "LineStyle",
    "LineWidth",
    "DisplayName",
    "Visible",
];

const SURFACE_PROPS: &[&str] = &[
    "FaceColor",
    "EdgeColor",
    "FaceAlpha",
    "LineStyle",
    "LineWidth",
];

const IMAGE_PROPS: &[&str] = &["CDataMapping", "Visible"];

fn canonical_prop(kind: DefaultObjectType, prop: &str) -> Option<&'static str> {
    let list = match kind {
        DefaultObjectType::Figure => FIGURE_PROPS,
        DefaultObjectType::Axes => AXES_PROPS,
        DefaultObjectType::Line => LINE_PROPS,
        DefaultObjectType::Text => TEXT_PROPS,
        DefaultObjectType::Patch => PATCH_PROPS,
        DefaultObjectType::Surface => SURFACE_PROPS,
        DefaultObjectType::Image => IMAGE_PROPS,
    };
    list.iter()
        .copied()
        .find(|candidate| candidate.eq_ignore_ascii_case(prop))
}

pub(in crate::engine) fn parse_default_or_factory(
    name: &str,
) -> Result<Option<(DefaultQueryKind, DefaultObjectType, &'static str)>> {
    let lower = name.to_ascii_lowercase();
    let (query_kind, rest) = if lower.starts_with("default") && name.len() > "default".len() {
        (DefaultQueryKind::Default, &name["default".len()..])
    } else if lower.starts_with("factory") && name.len() > "factory".len() {
        (DefaultQueryKind::Factory, &name["factory".len()..])
    } else {
        return Ok(None);
    };
    let Some((obj_type, prop)) = DefaultObjectType::from_prefix(rest) else {
        return Err(format!("Unknown root default property '{name}'"));
    };
    let Some(canon) = canonical_prop(obj_type, prop) else {
        return Err(format!(
            "Unknown property '{prop}' for {} default '{name}'",
            obj_type.as_str()
        ));
    };
    Ok(Some((query_kind, obj_type, canon)))
}

fn text_of(value: &Value) -> Result<&str> {
    match value {
        Value::Text(s) => Ok(s),
        _ => Err("Graphics property requires character text".into()),
    }
}

fn scalar_of(value: &Value, what: &str) -> Result<f64> {
    match value {
        Value::Matrix(m) if m.len() == 1 => {
            let v = m.as_scalar()?;
            if !v.is_finite() {
                return Err(format!("{what} requires a finite scalar"));
            }
            Ok(v)
        }
        Value::Logical(m) if m.len() == 1 => Ok(f64::from(m.as_scalar()?)),
        _ => Err(format!("{what} requires a real scalar")),
    }
}

fn vec_n(value: &Value, n: usize, what: &str) -> Result<Vec<f64>> {
    let Value::Matrix(m) = value else {
        return Err(format!("{what} requires a real vector"));
    };
    if m.len() != n || (m.rows() != 1 && m.cols() != 1) {
        return Err(format!("{what} requires {n} real values"));
    }
    Ok(m.data().to_vec())
}

fn on_off_of(value: &Value, what: &str) -> Result<bool> {
    match value {
        Value::Text(s) if s.eq_ignore_ascii_case("on") => Ok(true),
        Value::Text(s) if s.eq_ignore_ascii_case("off") => Ok(false),
        Value::Logical(m) => m.as_scalar(),
        Value::Matrix(m) if m.as_scalar()? == 1. => Ok(true),
        Value::Matrix(m) if m.as_scalar()? == 0. => Ok(false),
        _ => Err(format!("{what} requires 'on' or 'off'")),
    }
}

fn rgb_01(value: &Value, cancel: &AtomicBool) -> Result<[f64; 3]> {
    if let Value::Text(_) = value {
        let bytes = super::plot_properties::color(value)?;
        return Ok(bytes.map(|v| f64::from(v) / 255.));
    }
    super::graphics_handles::axes_rgb(value, cancel)
}

fn rgb_u8(value: &Value, cancel: &AtomicBool) -> Result<[u8; 3]> {
    let rgb = rgb_01(value, cancel)?;
    Ok(rgb.map(|v| (v * 255.).round() as u8))
}

fn line_pattern_of(value: &Value) -> Result<LinePattern> {
    match text_of(value)? {
        "-" => Ok(LinePattern::Solid),
        "--" => Ok(LinePattern::Dashed),
        ":" => Ok(LinePattern::Dotted),
        "-." => Ok(LinePattern::DashDot),
        "none" => Ok(LinePattern::None),
        _ => Err("LineStyle supports -, --, :, -. or none".into()),
    }
}

fn map_rows_of(value: &Value, limit: usize, what: &str) -> Result<Vec<[f64; 3]>> {
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

pub(in crate::engine) fn validate_default_value(
    obj_type: DefaultObjectType,
    prop: &str,
    value: &Value,
    cancel: &AtomicBool,
) -> Result<()> {
    match (obj_type, prop) {
        (DefaultObjectType::Figure, "Name") | (DefaultObjectType::Text, "String") => {
            if text_of(value)?.len() > 4096 {
                return Err(format!("{prop} exceeds 4096 UTF-8 bytes"));
            }
        }
        (DefaultObjectType::Line | DefaultObjectType::Patch, "DisplayName") => {
            if text_of(value)?.len() > 4096 {
                return Err("DisplayName exceeds 4096 UTF-8 bytes".into());
            }
        }
        (DefaultObjectType::Figure, "Color")
        | (DefaultObjectType::Axes, "Color" | "XColor" | "YColor" | "GridColor")
        | (DefaultObjectType::Line, "Color")
        | (DefaultObjectType::Text, "Color")
        | (DefaultObjectType::Patch, "FaceColor") => {
            rgb_01(value, cancel)?;
        }
        (DefaultObjectType::Line, "MarkerFaceColor") | (DefaultObjectType::Patch, "EdgeColor") => {
            if !matches!(value, Value::Text(s) if s.eq_ignore_ascii_case("none")) {
                rgb_01(value, cancel)?;
            }
        }
        (DefaultObjectType::Surface, "FaceColor" | "EdgeColor") => {
            if !matches!(value, Value::Text(s) if matches!(s.to_ascii_lowercase().as_str(), "flat" | "interp" | "none"))
            {
                rgb_01(value, cancel)?;
            }
        }
        (DefaultObjectType::Surface, "FaceAlpha") => {
            let a = scalar_of(value, "FaceAlpha")?;
            if !(0. ..=1.).contains(&a) {
                return Err("FaceAlpha must be in [0, 1]".into());
            }
        }
        (DefaultObjectType::Figure, "Theme") => {
            if !matches!(
                text_of(value)?.to_ascii_lowercase().as_str(),
                "light" | "dark"
            ) {
                return Err("Theme supports light or dark".into());
            }
        }
        (DefaultObjectType::Figure, "NumberTitle" | "Resize") => {
            on_off_of(value, prop)?;
        }
        (DefaultObjectType::Figure, "Units") => {
            crate::plot::FigureUnits::parse(text_of(value)?)?;
        }
        (DefaultObjectType::Figure, "WindowState") => {
            crate::plot::WindowState::parse(text_of(value)?)?;
        }
        (DefaultObjectType::Figure, "Position" | "InnerPosition" | "OuterPosition") => {
            let m = value.clone().matrix(cancel)?;
            if m.len() != 4 || (m.rows() != 1 && m.cols() != 1) {
                return Err(format!("Figure {prop} requires 4 real values"));
            }
            let d = m.data();
            if !d.iter().all(|v| v.is_finite()) || d[2] <= 0.0 || d[3] <= 0.0 {
                return Err(format!(
                    "Figure {prop} requires finite values with positive width and height"
                ));
            }
        }
        (DefaultObjectType::Figure, "ThemeMode" | "ColorMode")
        | (
            DefaultObjectType::Axes,
            "XColorMode" | "YColorMode" | "GridColorMode" | "GridAlphaMode",
        ) => {
            if !matches!(
                text_of(value)?.to_ascii_lowercase().as_str(),
                "auto" | "manual"
            ) {
                return Err(format!("{prop} supports auto or manual"));
            }
        }
        (DefaultObjectType::Figure, "InvertHardcopy" | "Visible")
        | (
            DefaultObjectType::Axes,
            "XGrid" | "YGrid" | "ZGrid" | "XMinorGrid" | "YMinorGrid" | "ZMinorGrid" | "Box",
        )
        | (
            DefaultObjectType::Line
            | DefaultObjectType::Text
            | DefaultObjectType::Patch
            | DefaultObjectType::Image,
            "Visible",
        ) => {
            on_off_of(value, prop)?;
        }
        (DefaultObjectType::Figure | DefaultObjectType::Axes, "Colormap") => {
            map_rows_of(value, crate::plot::MAX_COLORMAP_ROWS, "Colormap")?;
        }
        (DefaultObjectType::Axes, "ColorOrder") => {
            map_rows_of(value, crate::plot::MAX_COLOR_ORDER_ROWS, "ColorOrder")?;
        }
        (DefaultObjectType::Axes | DefaultObjectType::Text, "FontSize") => {
            let s = scalar_of(value, "FontSize")?;
            crate::plot::validate_font_size(s)?;
        }
        (DefaultObjectType::Axes, "GridAlpha") => {
            let a = scalar_of(value, "GridAlpha")?;
            if !(0. ..=1.).contains(&a) {
                return Err("GridAlpha requires a finite value in [0,1]".into());
            }
        }
        (DefaultObjectType::Axes, "XScale" | "YScale") => {
            if !matches!(
                text_of(value)?.to_ascii_lowercase().as_str(),
                "linear" | "log"
            ) {
                return Err("Scale must be linear or log".into());
            }
        }
        (DefaultObjectType::Axes, "NextPlot") => {
            if !matches!(text_of(value)?, "add" | "replace") {
                return Err("Axes NextPlot supports add or replace".into());
            }
        }
        (DefaultObjectType::Axes, "Position" | "InnerPosition" | "OuterPosition") => {
            let v = vec_n(value, 4, prop)?;
            crate::plot::Position::from_normalized([v[0], v[1], v[2], v[3]])?;
        }
        (DefaultObjectType::Axes, "View") => {
            let v = vec_n(value, 2, "View")?;
            crate::plot::view3d::validate_view([v[0], v[1]])?;
        }
        (DefaultObjectType::Axes, "Units") => {
            if !text_of(value)?.eq_ignore_ascii_case("normalized") {
                return Err("Axes Units supports normalized".into());
            }
        }
        (
            DefaultObjectType::Line | DefaultObjectType::Patch | DefaultObjectType::Surface,
            "LineWidth",
        ) => {
            let w = scalar_of(value, "LineWidth")?;
            crate::plot::validate_line_width(w)?;
        }
        (
            DefaultObjectType::Line | DefaultObjectType::Patch | DefaultObjectType::Surface,
            "LineStyle",
        ) => {
            line_pattern_of(value)?;
        }
        (DefaultObjectType::Line, "Marker") => {
            let s = text_of(value)?;
            if s != "none" {
                MarkerShape::parse(s)?;
            }
        }
        (DefaultObjectType::Line, "MarkerSize") => {
            let s = scalar_of(value, "MarkerSize")?;
            crate::plot::validate_marker_size(s)?;
        }
        (DefaultObjectType::Text, "FontWeight") => {
            if !matches!(
                text_of(value)?.to_ascii_lowercase().as_str(),
                "normal" | "bold"
            ) {
                return Err("FontWeight must be normal or bold".into());
            }
        }
        (DefaultObjectType::Text, "HorizontalAlignment") => {
            crate::plot::parse_horizontal(text_of(value)?)?;
        }
        (DefaultObjectType::Text, "VerticalAlignment") => {
            VerticalAlign::parse(text_of(value)?)?;
        }
        (DefaultObjectType::Text, "Rotation") => {
            scalar_of(value, "Rotation")?;
        }
        (DefaultObjectType::Text, "Units") => {
            if !text_of(value)?.eq_ignore_ascii_case("data") {
                return Err("Text Units supports data".into());
            }
        }
        (DefaultObjectType::Text, "Interpreter") => {
            if !matches!(
                text_of(value)?.to_ascii_lowercase().as_str(),
                "none" | "tex"
            ) {
                return Err("Text Interpreter supports none or tex".into());
            }
        }
        (DefaultObjectType::Image, "CDataMapping") => {
            if !matches!(
                text_of(value)?.to_ascii_lowercase().as_str(),
                "direct" | "scaled"
            ) {
                return Err("CDataMapping must be 'scaled' or 'direct'".into());
            }
        }
        _ => return Err(format!("Unsupported default property {prop}")),
    }
    Ok(())
}

fn map_to_value(rows: &[[f64; 3]]) -> Result<Value> {
    let mut data = Vec::with_capacity(rows.len() * 3);
    for c in 0..3 {
        data.extend(rows.iter().map(|r| r[c]));
    }
    Ok(Value::Matrix(Matrix::from_column_vec(rows.len(), 3, data)?))
}

pub(in crate::engine) fn factory_value(obj_type: DefaultObjectType, prop: &str) -> Result<Value> {
    let text = |s: &str| Ok(Value::Text(s.into()));
    let scalar = |v: f64| Ok(Value::Matrix(Matrix::scalar(v)));
    let rgb = |c: [f64; 3]| Ok(Value::Matrix(Matrix::from_column_slice(1, 3, &c)?));
    match (obj_type, prop) {
        (DefaultObjectType::Figure, "Name")
        | (DefaultObjectType::Line | DefaultObjectType::Patch, "DisplayName")
        | (DefaultObjectType::Text, "String") => text(""),
        (DefaultObjectType::Figure, "Color") | (DefaultObjectType::Axes, "Color") => {
            rgb([1.0, 1.0, 1.0])
        }
        (DefaultObjectType::Figure, "Theme") => text("light"),
        (DefaultObjectType::Figure, "Units") => text("pixels"),
        (DefaultObjectType::Figure, "WindowState") => text("normal"),
        (DefaultObjectType::Figure, "Position" | "InnerPosition" | "OuterPosition") => {
            Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                4,
                &crate::plot::DEFAULT_FIGURE_POSITION,
            )?))
        }
        (DefaultObjectType::Figure, "ThemeMode" | "ColorMode")
        | (
            DefaultObjectType::Axes,
            "XColorMode" | "YColorMode" | "GridColorMode" | "GridAlphaMode",
        ) => text("auto"),
        (DefaultObjectType::Figure, "InvertHardcopy" | "Visible" | "NumberTitle" | "Resize")
        | (DefaultObjectType::Axes, "Box")
        | (
            DefaultObjectType::Line
            | DefaultObjectType::Text
            | DefaultObjectType::Patch
            | DefaultObjectType::Image,
            "Visible",
        ) => text("on"),
        (DefaultObjectType::Figure | DefaultObjectType::Axes, "Colormap") => {
            map_to_value(&crate::plot::AxesExtras::default().effective_colormap())
        }
        (DefaultObjectType::Axes, "FontSize" | "TextFontSize") => scalar(10.0),
        (DefaultObjectType::Text, "FontSize") => scalar(TextObject::DEFAULT_FONT_SIZE),
        (DefaultObjectType::Axes, "XColor" | "YColor" | "GridColor") => rgb([0.15, 0.15, 0.15]),
        (DefaultObjectType::Axes, "GridAlpha") => scalar(0.15),
        (
            DefaultObjectType::Axes,
            "XGrid" | "YGrid" | "ZGrid" | "XMinorGrid" | "YMinorGrid" | "ZMinorGrid",
        ) => text("off"),
        (DefaultObjectType::Axes, "XScale" | "YScale") => text("linear"),
        (DefaultObjectType::Axes, "NextPlot") => text("replace"),
        (DefaultObjectType::Axes, "Position" | "InnerPosition") => Ok(Value::Matrix(
            Matrix::from_column_slice(1, 4, &[0.13, 0.11, 0.775, 0.815])?,
        )),
        (DefaultObjectType::Axes, "OuterPosition") => Ok(Value::Matrix(Matrix::from_column_slice(
            1,
            4,
            &[0.0, 0.0, 1.0, 1.0],
        )?)),
        (DefaultObjectType::Axes, "ColorOrder") => map_to_value(&automatic_color_order(
            crate::plot::PaletteVersion::MatRustModern1,
            crate::plot::BaseTheme::Light,
        )),
        (DefaultObjectType::Axes, "View") => Ok(Value::Matrix(Matrix::from_column_slice(
            1,
            2,
            &[0.0, 90.0],
        )?)),
        (DefaultObjectType::Axes, "Units") => text("normalized"),
        (
            DefaultObjectType::Line | DefaultObjectType::Patch | DefaultObjectType::Surface,
            "LineWidth",
        ) => scalar(0.5),
        (DefaultObjectType::Line, "Color") => rgb([0.0, 0.447, 0.741]),
        (
            DefaultObjectType::Line | DefaultObjectType::Patch | DefaultObjectType::Surface,
            "LineStyle",
        ) => text("-"),
        (DefaultObjectType::Line, "Marker" | "MarkerFaceColor") => text("none"),
        (DefaultObjectType::Line, "MarkerSize") => scalar(6.0),
        (DefaultObjectType::Text, "Color")
        | (DefaultObjectType::Patch, "FaceColor" | "EdgeColor")
        | (DefaultObjectType::Surface, "EdgeColor") => rgb([0.0, 0.0, 0.0]),
        (DefaultObjectType::Text, "FontWeight") => text("normal"),
        (DefaultObjectType::Text, "HorizontalAlignment") => text("left"),
        (DefaultObjectType::Text, "VerticalAlignment") => text("middle"),
        (DefaultObjectType::Text, "Rotation") => scalar(0.0),
        (DefaultObjectType::Text, "Units") => text("data"),
        (DefaultObjectType::Text, "Interpreter") => text("none"),
        (DefaultObjectType::Surface, "FaceColor") => text("flat"),
        (DefaultObjectType::Surface, "FaceAlpha") => scalar(1.0),
        (DefaultObjectType::Image, "CDataMapping") => text("direct"),
        _ => Err(format!("Unknown factory property {prop}")),
    }
}

fn defaults_struct(defaults: &GraphicsDefaults, cancel: &AtomicBool) -> Result<Value> {
    let mut names = Vec::with_capacity(defaults.len());
    let mut slots = Vec::with_capacity(defaults.len());
    for ((obj_type, prop), val) in defaults {
        names.push(format!("default{}{}", obj_type.as_str(), prop));
        slots.push(Arc::new(val.value.clone().array(cancel)?));
    }
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1, 1, names, slots, cancel,
    )?)))
}

impl Engine {
    pub(in crate::engine) fn reset_root(&mut self) {
        self.root = RootState::default();
    }

    pub(in crate::engine) fn is_figure_hidden(&self, id: &crate::ObjectIdentity) -> bool {
        self.hidden_figures.contains(&id.recovery_key())
    }

    pub(in crate::engine) fn set_figure_visible(
        &mut self,
        id: &crate::ObjectIdentity,
        visible: bool,
    ) {
        let key = id.recovery_key();
        if visible {
            self.hidden_figures.remove(&key);
        } else {
            self.hidden_figures.insert(key);
        }
    }

    pub(super) fn root_children_handles(&self, include_hidden: bool) -> Result<Value> {
        let mut figures: Vec<_> = self.figures().map(|(_, figure)| figure).collect();
        figures.sort_by_key(|figure| std::cmp::Reverse(figure.identity.creation_order()));
        let mut handles: Vec<_> = figures
            .into_iter()
            .map(|figure| Arc::new(GraphicsHandle::figure(&figure.identity)))
            .collect();
        let in_callback = !self.ui.callback_stack.is_empty();
        for &fk in &self.ui.figures {
            if let Some(node) = self.ui.get_key(fk)
                && (include_hidden
                    || self.root.show_hidden_handles
                    || node.handle_visibility.is_visible(in_callback))
            {
                handles.push(Arc::new(node.handle()));
            }
        }
        if include_hidden || self.root.show_hidden_handles {
            handles.extend(self.dialogs.hidden_handles().into_iter().map(Arc::new));
        }
        handles.sort_by_key(|h| std::cmp::Reverse(h.figure.creation_order()));
        let n = handles.len();
        Ok(Value::Container(
            GraphicsArray::from_shared(n, 1, handles, &self.cancel)?.into(),
        ))
    }

    pub(in crate::engine) fn get_root_property(&self, args: &[Value]) -> Result<Value> {
        if args.len() != 1 {
            return Err("get(0, ...) requires one property name".into());
        }
        let raw_name = text_of(&args[0])?;
        let lower = raw_name.to_ascii_lowercase();
        match lower.as_str() {
            "type" => return Ok(Value::Text("root".into())),
            "screensize" | "monitorpositions" => {
                return Ok(Value::Matrix(Matrix::from_column_slice(
                    1,
                    4,
                    &SCREEN_SIZE,
                )?));
            }
            "screenpixelsperinch" => return Ok(Value::Matrix(Matrix::scalar(SCREEN_PPI))),
            "units" => return Ok(Value::Text("pixels".into())),
            "children" => return self.root_children_handles(false),
            "parent" => {
                return Ok(Value::Container(
                    GraphicsArray::from_shared(0, 0, Vec::new(), &self.cancel)?.into(),
                ));
            }
            "currentfigure" => {
                return if let Some(fig) = &self.figure {
                    Ok(Value::Container(
                        GraphicsArray::scalar(GraphicsHandle::figure(&fig.identity), &self.cancel)?
                            .into(),
                    ))
                } else {
                    Ok(Value::Container(
                        GraphicsArray::from_shared(0, 0, Vec::new(), &self.cancel)?.into(),
                    ))
                };
            }
            "tag" => return Ok(Value::Text(self.root.tag.clone())),
            "userdata" => {
                return Ok(match &self.root.user_data {
                    Some(v) => v.clone(),
                    None => Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?),
                });
            }
            "showhiddenhandles" => {
                return Ok(Value::Text(
                    if self.root.show_hidden_handles {
                        "on"
                    } else {
                        "off"
                    }
                    .into(),
                ));
            }
            "default" => return defaults_struct(&self.root.defaults, &self.cancel),
            _ => {}
        }
        if let Some((query_kind, obj_type, canon)) = parse_default_or_factory(raw_name)? {
            return match query_kind {
                DefaultQueryKind::Default => match self.root.defaults.get(&(obj_type, canon)) {
                    Some(v) => Ok(v.value.clone()),
                    None => factory_value(obj_type, canon),
                },
                DefaultQueryKind::Factory => factory_value(obj_type, canon),
            };
        }
        Err(format!("Unsupported root property '{raw_name}'"))
    }

    pub(in crate::engine) fn set_root_properties(&mut self, args: &[Value]) -> Result<()> {
        if args.is_empty() || args.len() > 32 || !args.len().is_multiple_of(2) {
            return Err("set requires 1..16 property/value pairs".into());
        }
        for pair in args.as_chunks::<2>().0 {
            let raw_name = text_of(&pair[0])?;
            let lower = raw_name.to_ascii_lowercase();
            match lower.as_str() {
                "currentfigure" => {
                    let fig_id = match &pair[1] {
                        Value::Container(Array::Graphics(a)) => {
                            let h = a.scalar_handle()?;
                            if h.kind() != crate::GraphicsKind::Figure {
                                return Err("CurrentFigure must be a live figure handle".into());
                            }
                            self.locate_graphics(h)?.figure
                        }
                        Value::Matrix(m) => {
                            let n = m.as_scalar()?;
                            if !n.is_finite() || n.fract() != 0. || !(1. ..=1_000_000.).contains(&n)
                            {
                                return Err("CurrentFigure must be a live figure handle".into());
                            }
                            let id = n as u32;
                            if !self.figure_ids().contains(&id) {
                                return Err("CurrentFigure must be a live figure handle".into());
                            }
                            id
                        }
                        _ => return Err("CurrentFigure must be a live figure handle".into()),
                    };
                    self.select_figure(fig_id)?;
                    continue;
                }
                "units" => {
                    if !text_of(&pair[1])?.eq_ignore_ascii_case("pixels") {
                        return Err("Root Units supports only 'pixels'".into());
                    }
                    continue;
                }
                "tag" => {
                    let s = text_of(&pair[1])?;
                    if s.len() > 4096 {
                        return Err("Root Tag exceeds 4096 UTF-8 bytes".into());
                    }
                    self.root.tag = s.to_owned();
                    continue;
                }
                "userdata" => {
                    self.root.user_data = Some(pair[1].clone());
                    continue;
                }
                "showhiddenhandles" => {
                    self.root.show_hidden_handles = on_off_of(&pair[1], "ShowHiddenHandles")?;
                    continue;
                }
                "screensize"
                | "monitorpositions"
                | "screenpixelsperinch"
                | "children"
                | "type"
                | "parent" => {
                    return Err(format!("Root property '{raw_name}' is read-only"));
                }
                _ => {}
            }
            if let Some((query_kind, obj_type, canon)) = parse_default_or_factory(raw_name)? {
                if query_kind == DefaultQueryKind::Factory {
                    return Err(format!("Factory property '{raw_name}' is read-only"));
                }
                if matches!(&pair[1], Value::Text(s) if s.eq_ignore_ascii_case("remove")) {
                    self.root.defaults.remove(&(obj_type, canon));
                    continue;
                }
                let val = DefaultProperty::parse(obj_type, canon, &pair[1], &self.cancel)?;
                self.root.defaults.insert((obj_type, canon), val);
                continue;
            }
            return Err(format!("Unsupported root property '{raw_name}'"));
        }
        Ok(())
    }

    fn merged_defaults(
        &self,
        obj_type: DefaultObjectType,
        figure_id: Option<&crate::ObjectIdentity>,
    ) -> Vec<(&'static str, Value)> {
        let mut merged: BTreeMap<_, _> = self
            .root
            .defaults
            .iter()
            .filter(|((k, _), _)| *k == obj_type)
            .map(|((_, p), v)| (*p, v))
            .collect();
        if let Some(id) = figure_id
            && let Some(fig_map) = self.figure_defaults.get(&id.recovery_key())
        {
            for ((k, p), v) in fig_map {
                if *k == obj_type {
                    merged.insert(*p, v);
                }
            }
        }
        // Factory text size uses the object's implicit size. The merged map
        // still lets a figure's factory setting override a numeric root size.
        if obj_type == DefaultObjectType::Text
            && merged.get("FontSize").is_some_and(|v| v.is_factory)
        {
            merged.remove("FontSize");
        }
        merged
            .into_iter()
            .map(|(p, v)| (p, v.value.clone()))
            .collect()
    }

    pub(in crate::engine) fn apply_figure_defaults(&mut self, figure: &mut Figure) -> Result<()> {
        let entries = self.merged_defaults(DefaultObjectType::Figure, None);
        for (prop, val) in entries {
            match prop {
                "Name" => figure.name = text_of(&val)?.to_owned(),
                "InvertHardcopy" => figure.invert_hardcopy = Some(on_off_of(&val, prop)?),
                "Color" => {
                    figure.set_canvas_color(super::plot_properties::color(&val)?);
                }
                "Theme" => {
                    let t = match text_of(&val)?.to_ascii_lowercase().as_str() {
                        "light" => crate::plot::FigureTheme::Light,
                        _ => crate::plot::FigureTheme::Dark,
                    };
                    figure.set_figure_theme(t);
                    retheme_yyaxis(figure, self.graphics_theme_context);
                }
                "ThemeMode" => {
                    let m = if text_of(&val)?.eq_ignore_ascii_case("auto") {
                        ColorMode::Auto
                    } else {
                        ColorMode::Manual
                    };
                    figure.set_theme_mode(m, self.graphics_theme_context);
                    retheme_yyaxis(figure, self.graphics_theme_context);
                }
                "ColorMode" => {
                    let m = if text_of(&val)?.eq_ignore_ascii_case("auto") {
                        ColorMode::Auto
                    } else {
                        ColorMode::Manual
                    };
                    figure.set_canvas_mode(m, self.graphics_theme_context);
                }
                "Colormap" => {
                    let rows = Arc::new(map_rows_of(
                        &val,
                        crate::plot::MAX_COLORMAP_ROWS,
                        "Colormap",
                    )?);
                    for ax in figure.all_axes_mut() {
                        ax.extras.colormap = Some(rows.clone());
                    }
                }
                "Visible" => {
                    let vis = on_off_of(&val, "Visible")?;
                    figure.visible = vis;
                    self.set_figure_visible(&figure.identity, vis);
                }
                "NumberTitle" => figure.number_title = on_off_of(&val, "NumberTitle")?,
                "Resize" => figure.resize = on_off_of(&val, "Resize")?,
                "Units" => figure.units = crate::plot::FigureUnits::parse(text_of(&val)?)?,
                "WindowState" => {
                    figure.window_state = crate::plot::WindowState::parse(text_of(&val)?)?
                }
                "Position" | "InnerPosition" | "OuterPosition" => {
                    let m = val.clone().matrix(&self.cancel)?;
                    if m.len() != 4 || (m.rows() != 1 && m.cols() != 1) {
                        return Err(format!("Figure {prop} requires 4 real values"));
                    }
                    let d = m.data();
                    figure.set_position_in_units([d[0], d[1], d[2], d[3]])?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(in crate::engine) fn apply_axes_defaults(&mut self, figure: &mut Figure) -> Result<()> {
        self.graphics_new_axes_defaults(figure, false)
    }

    pub(in crate::engine) fn graphics_new_axes_defaults(
        &mut self,
        figure: &mut Figure,
        pending_figure_axes: bool,
    ) -> Result<()> {
        let entries = self.merged_defaults(DefaultObjectType::Axes, Some(&figure.identity));
        self.graphics_apply_axes_default_values(figure, &entries, |_| true)?;
        self.graphics_record_axes_defaults(figure, entries, pending_figure_axes);
        Ok(())
    }

    fn graphics_record_axes_defaults(
        &mut self,
        figure: &Figure,
        values: AxesDefaultValues,
        pending_figure_axes: bool,
    ) {
        let key = figure.axes.identity.creation_order();
        if values.is_empty() && !pending_figure_axes {
            self.axes_creation_defaults.remove(&key);
            return;
        }
        // Discard closed and replaced axes periodically. Stale handles never
        // address these entries because creation orders are unique.
        if self.axes_creation_defaults.len() >= 256 {
            let live: std::collections::BTreeSet<_> = self
                .figures()
                .flat_map(|(_, f)| f.all_axes().map(|(_, a)| a.identity.creation_order()))
                .collect();
            self.axes_creation_defaults
                .retain(|id, _| live.contains(id));
        }
        self.axes_creation_defaults.insert(
            key,
            AxesCreationDefaults {
                values,
                pending_figure_axes,
            },
        );
    }

    pub(in crate::engine) fn graphics_axes_defaults_pending(&self, figure: &Figure) -> bool {
        self.axes_creation_defaults
            .get(&figure.axes.identity.creation_order())
            .is_some_and(|defaults| defaults.pending_figure_axes)
    }

    pub(in crate::engine) fn graphics_plot_axes_defaults(
        &mut self,
        figure: &mut Figure,
    ) -> Result<Option<AxesDefaultValues>> {
        let Some(previous) = &self.figure else {
            self.apply_figure_defaults(figure)?;
            let values = self.merged_defaults(DefaultObjectType::Axes, Some(&figure.identity));
            self.graphics_apply_axes_default_values(figure, &values, |_| true)?;
            return Ok(Some(values));
        };
        self.graphics_existing_axes_defaults(figure, Some(previous))
    }

    pub(in crate::engine) fn graphics_existing_axes_defaults(
        &self,
        figure: &mut Figure,
        previous_plot: Option<&Figure>,
    ) -> Result<Option<AxesDefaultValues>> {
        let Some(creation) = self
            .axes_creation_defaults
            .get(&figure.axes.identity.creation_order())
        else {
            return Ok(None);
        };
        let previous = previous_plot.unwrap_or(figure);
        let replacing = previous_plot.is_some() && !previous.hold;
        let replacing_yy = replacing && previous.axes.extras.yy.is_some();
        // A bare figure holds an empty axes until it is used. Keep its root
        // defaults from figure creation and resolve its figure-local overrides
        // when the axes is first exposed or drawn into.
        let overrides: AxesDefaultValues = if creation.pending_figure_axes
            && previous.axes.series.is_empty()
            && previous.axes.extras.images.is_empty()
        {
            self.figure_defaults
                .get(&figure.identity.recovery_key())
                .into_iter()
                .flatten()
                .filter(|((kind, _), _)| *kind == DefaultObjectType::Axes)
                .map(|((_, prop), value)| (*prop, value.value.clone()))
                .collect()
        } else {
            Vec::new()
        };
        let resolved = creation.pending_figure_axes.then(|| {
            let mut values: BTreeMap<_, _> = creation.values.iter().cloned().collect();
            values.extend(overrides.iter().cloned());
            values.into_iter().collect()
        });
        if replacing {
            self.graphics_apply_axes_default_values(
                figure,
                resolved.as_deref().unwrap_or(&creation.values),
                |prop| match prop {
                    // The live colormap and hold state already survive a plot.
                    "Colormap" | "NextPlot" => overrides.iter().any(|(name, _)| *name == prop),
                    "XScale" => true,
                    _ => !replacing_yy,
                },
            )?;
        } else {
            self.graphics_apply_axes_default_values(figure, &overrides, |_| true)?;
        }
        Ok(resolved)
    }

    pub(in crate::engine) fn graphics_finish_axes_defaults(
        &mut self,
        figure: &Figure,
        values: Option<AxesDefaultValues>,
    ) {
        if let Some(values) = values {
            self.graphics_record_axes_defaults(figure, values, false);
        }
    }

    fn graphics_apply_axes_default_values(
        &self,
        figure: &mut Figure,
        entries: &[(&str, Value)],
        includes: impl Fn(&str) -> bool,
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let appearance = figure.resolve_appearance(self.graphics_theme_context);
        let axes = &mut figure.axes;
        let modern = !appearance.legacy;
        for (prop, val) in entries.iter().filter(|(prop, _)| includes(prop)) {
            let prop = *prop;
            match prop {
                "FontSize" => axes.extras.font_size = Some(scalar_of(val, prop)?),
                "Color" => {
                    axes.colors.get_or_insert_with(Default::default).background =
                        Some(rgb_01(val, &self.cancel)?);
                }
                "XColor" => {
                    let colors = axes.colors.get_or_insert_with(Default::default);
                    colors.x = Some(rgb_01(val, &self.cancel)?);
                    colors
                        .grid
                        .get_or_insert_with(|| AxesGridStyle::from_appearance(appearance, modern))
                        .x_mode = ColorMode::Manual;
                }
                "YColor" => {
                    let colors = axes.colors.get_or_insert_with(Default::default);
                    colors.y = Some(rgb_01(val, &self.cancel)?);
                    colors
                        .grid
                        .get_or_insert_with(|| AxesGridStyle::from_appearance(appearance, modern))
                        .y_mode = ColorMode::Manual;
                }
                "GridColor" => {
                    let colors = axes.colors.get_or_insert_with(Default::default);
                    let grid = colors
                        .grid
                        .get_or_insert_with(|| AxesGridStyle::from_appearance(appearance, modern));
                    grid.color = rgb_01(val, &self.cancel)?;
                    grid.color_mode = ColorMode::Manual;
                }
                "GridAlpha" => {
                    let colors = axes.colors.get_or_insert_with(Default::default);
                    let grid = colors
                        .grid
                        .get_or_insert_with(|| AxesGridStyle::from_appearance(appearance, modern));
                    grid.alpha = scalar_of(val, prop)?;
                    grid.alpha_mode = ColorMode::Manual;
                }
                "XColorMode" | "YColorMode" | "GridColorMode" | "GridAlphaMode" => {
                    let mode = if text_of(val)?.eq_ignore_ascii_case("auto") {
                        ColorMode::Auto
                    } else {
                        ColorMode::Manual
                    };
                    let colors = axes.colors.get_or_insert_with(Default::default);
                    let grid = colors
                        .grid
                        .get_or_insert_with(|| AxesGridStyle::from_appearance(appearance, modern));
                    match prop {
                        "XColorMode" => grid.x_mode = mode,
                        "YColorMode" => grid.y_mode = mode,
                        "GridColorMode" => grid.color_mode = mode,
                        _ => grid.alpha_mode = mode,
                    }
                }
                "XGrid" => axes.xgrid = on_off_of(val, prop)?,
                "YGrid" => axes.ygrid = on_off_of(val, prop)?,
                "ZGrid" => axes.zgrid = on_off_of(val, prop)?,
                "XMinorGrid" => axes.minor_grid[0] = on_off_of(val, prop)?,
                "YMinorGrid" => axes.minor_grid[1] = on_off_of(val, prop)?,
                "ZMinorGrid" => axes.minor_grid[2] = on_off_of(val, prop)?,
                "Box" => axes.box_outline = on_off_of(val, prop)?,
                "XScale" => {
                    axes.xscale = if text_of(val)?.eq_ignore_ascii_case("log") {
                        crate::plot::AxisScale::Log
                    } else {
                        crate::plot::AxisScale::Linear
                    };
                }
                "YScale" => {
                    axes.yscale = if text_of(val)?.eq_ignore_ascii_case("log") {
                        crate::plot::AxisScale::Log
                    } else {
                        crate::plot::AxisScale::Linear
                    };
                }
                "NextPlot" => axes.hold = text_of(val)? == "add",
                "ColorOrder" => {
                    axes.extras.color_order = Some(Arc::new(map_rows_of(
                        val,
                        crate::plot::MAX_COLOR_ORDER_ROWS,
                        "ColorOrder",
                    )?));
                }
                "Colormap" => {
                    axes.extras.colormap = Some(Arc::new(map_rows_of(
                        val,
                        crate::plot::MAX_COLORMAP_ROWS,
                        "Colormap",
                    )?));
                }
                "View" => {
                    let v = vec_n(val, 2, "View")?;
                    axes.view = [v[0], v[1]];
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(in crate::engine) fn apply_line_defaults(
        &self,
        batch: &mut [Series],
        figure_id: Option<&crate::ObjectIdentity>,
        primitive_line: bool,
    ) -> Result<()> {
        let entries = self.merged_defaults(DefaultObjectType::Line, figure_id);
        if entries.is_empty() {
            return Ok(());
        }
        for s in batch.iter_mut() {
            if s.kind != PlotKind::Line {
                continue;
            }
            for (prop, val) in &entries {
                match *prop {
                    "LineWidth" => s.line_width = Some(scalar_of(val, prop)?),
                    // High-level plots assign their own automatic colors.
                    "Color" if primitive_line => {
                        s.color = rgb_u8(val, &self.cancel)?;
                        s.explicit_color = true;
                    }
                    "LineStyle" => s.pattern = line_pattern_of(val)?,
                    "Marker" => {
                        let m = text_of(val)?;
                        if m == "none" {
                            s.marker_shape = None;
                            s.markers = false;
                        } else {
                            let shape = MarkerShape::parse(m)?;
                            s.marker_shape = Some(shape);
                            s.markers = true;
                            if shape == MarkerShape::Point {
                                s.marker_filled = true;
                            }
                        }
                    }
                    "MarkerSize" => s.marker_size = Some(scalar_of(val, prop)?),
                    "MarkerFaceColor" => {
                        s.marker_filled =
                            !matches!(val, Value::Text(t) if t.eq_ignore_ascii_case("none"));
                    }
                    "DisplayName" => s.label = text_of(val)?.to_owned(),
                    "Visible" => s.visible = on_off_of(val, prop)?,
                    _ => {}
                }
            }
        }
        Ok(())
    }

    pub(in crate::engine) fn apply_text_defaults(
        &self,
        object: &mut TextObject,
        figure_id: &crate::ObjectIdentity,
    ) -> Result<()> {
        for (prop, val) in self.merged_defaults(DefaultObjectType::Text, Some(figure_id)) {
            match prop {
                "String" => object.string = text_of(&val)?.to_owned(),
                "Color" => object.color = Some(rgb_01(&val, &self.cancel)?),
                "FontSize" => {
                    object.font_size = scalar_of(&val, prop)?;
                    object.font_size_explicit = true;
                }
                "FontWeight" => object.bold = text_of(&val)?.eq_ignore_ascii_case("bold"),
                "HorizontalAlignment" => {
                    object.horizontal = crate::plot::parse_horizontal(text_of(&val)?)?
                }
                "VerticalAlignment" => object.vertical = VerticalAlign::parse(text_of(&val)?)?,
                "Rotation" => object.rotation = scalar_of(&val, prop)?,
                "Visible" => object.visible = on_off_of(&val, prop)?,
                _ => {}
            }
        }
        Ok(())
    }

    pub(in crate::engine) fn apply_patch_defaults(
        &self,
        series: &mut Series,
        figure_id: &crate::ObjectIdentity,
    ) -> Result<()> {
        for (prop, val) in self.merged_defaults(DefaultObjectType::Patch, Some(figure_id)) {
            match prop {
                "FaceColor" => {
                    series.color = rgb_u8(&val, &self.cancel)?;
                    series.explicit_color = true;
                }
                "EdgeColor" => {
                    series.face_edge_color = if matches!(&val, Value::Text(s) if s.eq_ignore_ascii_case("none"))
                    {
                        None
                    } else {
                        Some(rgb_u8(&val, &self.cancel)?)
                    };
                }
                "LineStyle" => series.pattern = line_pattern_of(&val)?,
                "LineWidth" => series.line_width = Some(scalar_of(&val, prop)?),
                "DisplayName" => series.label = text_of(&val)?.to_owned(),
                "Visible" => series.visible = on_off_of(&val, prop)?,
                _ => {}
            }
        }
        Ok(())
    }

    pub(in crate::engine) fn apply_image_defaults(
        &self,
        image: &mut ImageObject,
        figure_id: &crate::ObjectIdentity,
    ) -> Result<()> {
        for (prop, val) in self.merged_defaults(DefaultObjectType::Image, Some(figure_id)) {
            match prop {
                "Visible" => image.visible = on_off_of(&val, prop)?,
                "CDataMapping" if !matches!(image.mapping, ImageMapping::TrueColor { .. }) => {
                    image.mapping = if text_of(&val)?.eq_ignore_ascii_case("scaled") {
                        ImageMapping::Scaled
                    } else {
                        ImageMapping::Direct { zero_based: false }
                    };
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(in crate::engine) fn apply_surface_defaults(
        &self,
        figure_id: &crate::ObjectIdentity,
    ) -> Vec<(&'static str, Value)> {
        self.merged_defaults(DefaultObjectType::Surface, Some(figure_id))
    }
}
