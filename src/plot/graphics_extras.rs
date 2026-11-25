//! Axes-owned graphics objects beyond series: text, colorbar, colormap,
//! colour limits, font size, colour order and a second (right) y-axis.
//!
//! These records live in one separate struct on `Axes` so that series kinds
//! (`PlotKind`) stay untouched. They are process-local document state: native
//! `.fig`/session files do not store them yet.
use super::{Axes, AxisScale, LinePattern, TextAlign};
use crate::{ObjectIdentity, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const MAX_TEXTS: usize = 256;
pub const MAX_CONSTANT_LINES: usize = 256;
pub const MAX_RECTANGLES: usize = 256;
pub const MAX_TEXT_BYTES: usize = 4096;
pub const MAX_COLORMAP_ROWS: usize = 4096;
pub const MAX_COLOR_ORDER_ROWS: usize = 256;

/// Vertical anchor of a text object relative to its position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerticalAlign {
    #[default]
    Middle,
    Top,
    Bottom,
    Baseline,
    Cap,
}
impl VerticalAlign {
    pub fn parse(text: &str) -> Result<Self> {
        Ok(match text.to_ascii_lowercase().as_str() {
            "middle" => Self::Middle,
            "top" => Self::Top,
            "bottom" => Self::Bottom,
            "baseline" => Self::Baseline,
            "cap" => Self::Cap,
            _ => {
                return Err(
                    "VerticalAlignment must be middle, top, bottom, baseline or cap".into(),
                );
            }
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Middle => "middle",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Baseline => "baseline",
            Self::Cap => "cap",
        }
    }
}
pub fn parse_horizontal(text: &str) -> Result<TextAlign> {
    Ok(match text.to_ascii_lowercase().as_str() {
        "left" => TextAlign::Left,
        "center" => TextAlign::Center,
        "right" => TextAlign::Right,
        _ => return Err("HorizontalAlignment must be left, center or right".into()),
    })
}
pub fn horizontal_name(align: TextAlign) -> &'static str {
    match align {
        TextAlign::Left => "left",
        TextAlign::Center => "center",
        TextAlign::Right => "right",
    }
}

/// Axis intercepted by a `ConstantLineObject` (`xline` vs `yline`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterceptAxis {
    X,
    Y,
}
impl InterceptAxis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
        }
    }
}

/// Horizontal label alignment for `xline`/`yline`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstantLineHAlign {
    Left,
    Center,
    #[default]
    Right,
}
impl ConstantLineHAlign {
    pub fn parse(text: &str) -> Result<Self> {
        match text.to_ascii_lowercase().as_str() {
            "left" => Ok(Self::Left),
            "center" => Ok(Self::Center),
            "right" => Ok(Self::Right),
            _ => Err("LabelHorizontalAlignment must be left, center or right".into()),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

/// Vertical label alignment for `xline`/`yline`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstantLineVAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}
impl ConstantLineVAlign {
    pub fn parse(text: &str) -> Result<Self> {
        match text.to_ascii_lowercase().as_str() {
            "top" => Ok(Self::Top),
            "middle" => Ok(Self::Middle),
            "bottom" => Ok(Self::Bottom),
            _ => Err("LabelVerticalAlignment must be top, middle or bottom".into()),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Middle => "middle",
            Self::Bottom => "bottom",
        }
    }
}

/// A constant horizontal or vertical line (`xline`, `yline`) that spans the axes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstantLineObject {
    #[serde(skip)]
    pub identity: ObjectIdentity,
    pub axis: InterceptAxis,
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f64; 3]>,
    pub pattern: LinePattern,
    pub line_width: f64,
    pub alpha: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default)]
    pub label_h_align: ConstantLineHAlign,
    #[serde(default)]
    pub label_v_align: ConstantLineVAlign,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    #[serde(default = "default_true")]
    pub visible: bool,
}
fn default_true() -> bool {
    true
}
impl ConstantLineObject {
    pub fn new(axis: InterceptAxis, value: f64) -> Self {
        Self {
            identity: ObjectIdentity::default(),
            axis,
            value,
            color: None,
            pattern: LinePattern::Solid,
            line_width: 0.5,
            alpha: 0.7,
            label: String::new(),
            label_h_align: ConstantLineHAlign::Right,
            label_v_align: ConstantLineVAlign::Top,
            display_name: String::new(),
            visible: true,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if !self.value.is_finite() {
            return Err("ConstantLine Value must be a finite scalar".into());
        }
        if let Some(rgb) = self.color {
            validate_rgb(rgb)?;
        }
        super::validate_line_width(self.line_width)?;
        if !self.alpha.is_finite() || !(0. ..=1.).contains(&self.alpha) {
            return Err("ConstantLine Alpha must be a finite scalar in 0..1".into());
        }
        if self.label.len() > MAX_TEXT_BYTES || self.display_name.len() > MAX_TEXT_BYTES {
            return Err("ConstantLine label exceeds 4096 UTF-8 bytes".into());
        }
        Ok(())
    }
    fn same(&self, other: &Self) -> bool {
        self.axis == other.axis
            && self.value.to_bits() == other.value.to_bits()
            && self.color == other.color
            && self.pattern == other.pattern
            && self.line_width.to_bits() == other.line_width.to_bits()
            && self.alpha.to_bits() == other.alpha.to_bits()
            && self.label == other.label
            && self.label_h_align == other.label_h_align
            && self.label_v_align == other.label_v_align
            && self.display_name == other.display_name
            && self.visible == other.visible
    }
}

/// A 2-D rectangle or ellipse (`rectangle`) in data coordinates.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RectangleObject {
    #[serde(skip)]
    pub identity: ObjectIdentity,
    pub position: [f64; 4],
    pub curvature: [f64; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_color: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_color: Option<[f64; 3]>,
    pub line_width: f64,
    pub pattern: LinePattern,
    #[serde(default = "default_true")]
    pub visible: bool,
}
impl Default for RectangleObject {
    fn default() -> Self {
        Self::new()
    }
}
impl RectangleObject {
    pub fn new() -> Self {
        Self {
            identity: ObjectIdentity::default(),
            position: [0., 0., 1., 1.],
            curvature: [0., 0.],
            face_color: None,
            edge_color: Some([0., 0., 0.]),
            line_width: 0.5,
            pattern: LinePattern::Solid,
            visible: true,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.position.iter().any(|v| !v.is_finite())
            || self.position[2] < 0.
            || self.position[3] < 0.
        {
            return Err(
                "Rectangle Position requires [x y w h] with finite values and w,h >= 0".into(),
            );
        }
        if self
            .curvature
            .iter()
            .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
        {
            return Err("Rectangle Curvature values must be finite in 0..1".into());
        }
        if let Some(rgb) = self.face_color {
            validate_rgb(rgb)?;
        }
        if let Some(rgb) = self.edge_color {
            validate_rgb(rgb)?;
        }
        super::validate_line_width(self.line_width)?;
        Ok(())
    }
    fn same(&self, other: &Self) -> bool {
        self.position.map(f64::to_bits) == other.position.map(f64::to_bits)
            && self.curvature.map(f64::to_bits) == other.curvature.map(f64::to_bits)
            && self.face_color == other.face_color
            && self.edge_color == other.edge_color
            && self.line_width.to_bits() == other.line_width.to_bits()
            && self.pattern == other.pattern
            && self.visible == other.visible
    }
}

/// A `text(x, y, str)` object in data coordinates.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextObject {
    #[serde(skip)]
    pub identity: ObjectIdentity,
    pub string: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub string_lines: Option<super::text_markup::TextSource>,
    #[serde(default)]
    pub interpreter: super::text_markup::Interpreter,
    pub position: [f64; 3],
    /// None follows the axes text colour of the resolved theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f64; 3]>,
    /// Typographic points.
    pub font_size: f64,
    #[serde(default)]
    pub font_size_explicit: bool,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub horizontal: TextAlign,
    #[serde(default)]
    pub vertical: VerticalAlign,
    /// Degrees, counterclockwise.
    #[serde(default)]
    pub rotation: f64,
    #[serde(default = "default_true")]
    pub visible: bool,
}
impl TextObject {
    pub(crate) const DEFAULT_FONT_SIZE: f64 = 10.;

    pub fn new(x: f64, y: f64, string: String) -> Self {
        Self {
            identity: ObjectIdentity::default(),
            string,
            string_lines: None,
            interpreter: Default::default(),
            position: [x, y, 0.],
            color: None,
            font_size: Self::DEFAULT_FONT_SIZE,
            font_size_explicit: false,
            bold: false,
            italic: false,
            horizontal: TextAlign::Left,
            vertical: VerticalAlign::Middle,
            rotation: 0.,
            visible: true,
        }
    }
    fn same(&self, other: &Self) -> bool {
        self.string == other.string
            && self.string_lines == other.string_lines
            && self.interpreter == other.interpreter
            && self.position == other.position
            && self.color == other.color
            && self.font_size == other.font_size
            && self.font_size_explicit == other.font_size_explicit
            && self.bold == other.bold
            && self.italic == other.italic
            && self.horizontal == other.horizontal
            && self.vertical == other.vertical
            && self.rotation == other.rotation
            && self.visible == other.visible
    }
    pub fn validate(&self) -> Result<()> {
        if self.string.len() > MAX_TEXT_BYTES {
            return Err("Text String exceeds 4096 UTF-8 bytes".into());
        }
        if !self.position.iter().all(|v| v.is_finite()) {
            return Err("Text Position must be finite".into());
        }
        validate_font_size(self.font_size)?;
        if !self.rotation.is_finite() {
            return Err("Text Rotation must be finite".into());
        }
        if let Some(rgb) = self.color {
            validate_rgb(rgb)?;
        }
        Ok(())
    }
}

/// Where a colorbar sits relative to its axes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorbarLocation {
    #[default]
    EastOutside,
    SouthOutside,
}
impl ColorbarLocation {
    pub fn parse(text: &str) -> Result<Self> {
        Ok(match text.to_ascii_lowercase().as_str() {
            "eastoutside" | "east" | "vertical" => Self::EastOutside,
            "southoutside" | "south" | "horizontal" => Self::SouthOutside,
            _ => {
                return Err(
                    "Colorbar Location supports eastoutside or southoutside (vertical or horizontal)"
                        .into(),
                );
            }
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EastOutside => "eastoutside",
            Self::SouthOutside => "southoutside",
        }
    }
    pub fn horizontal(self) -> bool {
        self == Self::SouthOutside
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Colorbar {
    #[serde(skip)]
    pub identity: ObjectIdentity,
    #[serde(skip)]
    pub label_identity: ObjectIdentity,
    pub label: String,
    #[serde(skip)]
    pub title_identity: ObjectIdentity,
    pub title: String,
    pub location: ColorbarLocation,
    pub visible: bool,
    pub font_size: Option<f64>,
}
impl Colorbar {
    pub fn new(location: ColorbarLocation) -> Self {
        Self {
            location,
            visible: true,
            ..Self::default()
        }
    }
    fn same(&self, other: &Self) -> bool {
        self.label == other.label
            && self.title == other.title
            && self.location == other.location
            && self.visible == other.visible
            && self.font_size == other.font_size
    }
}

/// The right-hand y-axis created by `yyaxis`.
#[derive(Clone, Debug)]
pub struct YyAxis {
    pub active_right: bool,
    /// Series identities that belong to the right side; all others are left.
    pub right: Vec<ObjectIdentity>,
    pub left_color: [f64; 3],
    pub right_color: [f64; 3],
    pub left_explicit: bool,
    pub right_explicit: bool,
    pub right_ylim: Option<[f64; 2]>,
    pub right_label: String,
    pub right_label_style: LabelStyle,
    pub right_label_identity: ObjectIdentity,
    /// Set only on a display copy: resolved (left, right) limits.
    pub resolved: Option<[[f64; 2]; 2]>,
}
impl YyAxis {
    pub fn new(order: &[[f64; 3]]) -> Self {
        Self {
            active_right: false,
            right: Vec::new(),
            left_color: order[0],
            right_color: order[1 % order.len()],
            left_explicit: false,
            right_explicit: false,
            right_ylim: None,
            right_label: String::new(),
            right_label_style: LabelStyle::default(),
            right_label_identity: ObjectIdentity::default(),
            resolved: None,
        }
    }
    pub fn is_right(&self, id: &ObjectIdentity) -> bool {
        self.right.iter().any(|r| r.same_object(id))
    }
    fn same(&self, other: &Self) -> bool {
        self.active_right == other.active_right
            && self.right.len() == other.right.len()
            && self
                .right
                .iter()
                .zip(&other.right)
                .all(|(a, b)| a.same_object(b))
            && self.left_color == other.left_color
            && self.right_color == other.right_color
            && self.left_explicit == other.left_explicit
            && self.right_explicit == other.right_explicit
            && self.right_ylim == other.right_ylim
            && self.right_label == other.right_label
            && self.right_label_style == other.right_label_style
            && self.resolved == other.resolved
    }
}

/// Explicit title or label text styling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LabelStyle {
    pub font_size: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
}

impl LabelStyle {
    pub fn font_size_pixels(self, default: f32) -> f32 {
        self.font_size
            .map_or(default, |size| (size * POINTS_TO_PIXELS) as f32)
    }

    pub fn export_font_size_pixels(self, default: f32, px_per_point: f32) -> f32 {
        self.font_size
            .map_or(default, |size| size as f32 * px_per_point)
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub(super) fn validate(self) -> Result<()> {
        self.font_size.map_or(Ok(()), validate_font_size)
    }
}

pub const MAX_TICKS: usize = 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TickDir {
    #[default]
    In,
    Out,
    Both,
    None,
}

impl TickDir {
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "in" => Ok(Self::In),
            "out" => Ok(Self::Out),
            "both" => Ok(Self::Both),
            "none" => Ok(Self::None),
            _ => Err("TickDir must be in, out, both or none".into()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
            Self::Both => "both",
            Self::None => "none",
        }
    }
}

/// Axes `Layer`: whether grid lines and tick marks are painted under
/// (`bottom`, the default) or over (`top`) the axes children.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AxesLayer {
    #[default]
    Bottom,
    Top,
}

impl AxesLayer {
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "bottom" => Ok(Self::Bottom),
            "top" => Ok(Self::Top),
            _ => Err("Layer must be 'bottom' or 'top'".into()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bottom => "bottom",
            Self::Top => "top",
        }
    }

    pub fn is_top(self) -> bool {
        self == Self::Top
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum RulerKind {
    #[default]
    Numeric,
    Datetime {
        #[serde(default, skip_serializing_if = "is_false")]
        utc: bool,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        format: String,
    },
    Duration {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        format: String,
        #[serde(default = "default_unit_ms")]
        unit_ms: f64,
    },
    Categorical {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        categories: Vec<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        ordinal: bool,
    },
}

fn is_false(v: &bool) -> bool {
    !*v
}

fn default_unit_ms() -> f64 {
    1_000.0
}

impl RulerKind {
    pub fn is_numeric(&self) -> bool {
        matches!(self, Self::Numeric)
    }

    pub fn class_name(&self) -> &'static str {
        match self {
            Self::Numeric => "matlab.graphics.axis.decorator.NumericRuler",
            Self::Datetime { .. } => "matlab.graphics.axis.decorator.DatetimeRuler",
            Self::Duration { .. } => "matlab.graphics.axis.decorator.DurationRuler",
            Self::Categorical { .. } => "matlab.graphics.axis.decorator.CategoricalRuler",
        }
    }

    pub fn short_class_name(&self) -> &'static str {
        match self {
            Self::Numeric => "NumericRuler",
            Self::Datetime { .. } => "DatetimeRuler",
            Self::Duration { .. } => "DurationRuler",
            Self::Categorical { .. } => "CategoricalRuler",
        }
    }

    pub fn kind_word(&self) -> &'static str {
        match self {
            Self::Numeric => "numeric",
            Self::Datetime { .. } => "datetime",
            Self::Duration { .. } => "duration",
            Self::Categorical { .. } => "categorical",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxisRuler {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticks: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_labels: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub tick_label_rotation: f64,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub exponent: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_label_format: Option<String>,
    #[serde(default, skip_serializing_if = "RulerKind::is_numeric")]
    pub kind: RulerKind,
}

fn is_zero_f64(v: &f64) -> bool {
    *v == 0.
}

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

pub fn validate_ticks(ticks: &[f64]) -> Result<()> {
    if ticks.len() > MAX_TICKS {
        return Err(format!("Ticks exceed {MAX_TICKS} entries"));
    }
    if ticks.iter().any(|v| !v.is_finite()) {
        return Err("Ticks must be finite".into());
    }
    if ticks.windows(2).any(|w| w[0] >= w[1]) {
        return Err("Ticks must be strictly increasing".into());
    }
    Ok(())
}

pub fn validate_tick_labels(labels: &[String]) -> Result<()> {
    if labels.len() > MAX_TICKS {
        return Err(format!("TickLabels exceed {MAX_TICKS} entries"));
    }
    if labels.iter().any(|s| s.len() > MAX_TEXT_BYTES) {
        return Err("Tick label exceeds 4096 UTF-8 bytes".into());
    }
    Ok(())
}

pub fn validate_tick_length(len: [f64; 2]) -> Result<()> {
    if len
        .iter()
        .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
    {
        return Err("TickLength values must be finite in [0, 1]".into());
    }
    Ok(())
}

impl AxisRuler {
    pub fn validate(&self) -> Result<()> {
        if let Some(ticks) = &self.ticks {
            validate_ticks(ticks)?;
        }
        if let Some(labels) = &self.tick_labels {
            validate_tick_labels(labels)?;
        }
        if !self.tick_label_rotation.is_finite() {
            return Err("TickLabelRotation must be finite".into());
        }
        if !(-308..=308).contains(&self.exponent) {
            return Err("Exponent must be an integer in -308..=308".into());
        }
        if let Some(fmt) = &self.tick_label_format {
            if fmt.len() > MAX_TEXT_BYTES {
                return Err("TickLabelFormat exceeds 4096 UTF-8 bytes".into());
            }
            crate::engine::text_format::format_items(
                "TickLabelFormat",
                fmt,
                &[crate::engine::text_format::Item::Num(1.0)],
                &std::sync::atomic::AtomicBool::new(false),
            )?;
        }
        Ok(())
    }
}

/// Text in the heading band of an axes or tiled layout.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Subtitle {
    pub string: String,
    pub string_lines: Option<super::text_markup::TextSource>,
    pub interpreter: super::text_markup::Interpreter,
    pub color: Option<[f64; 3]>,
    pub font_size: Option<f64>,
    pub bold: bool,
    pub italic: bool,
}

const POINTS_TO_PIXELS: f64 = 4. / 3.;

impl Subtitle {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// MATLAB's default subtitle size is the axes FontSize, 10 pt.
    pub const DEFAULT_POINTS: f64 = 10.;

    pub fn font_size_points(&self) -> f64 {
        self.font_size.unwrap_or(Self::DEFAULT_POINTS)
    }

    /// Each renderer passes its own pixels per point, so the 10 pt default keeps its
    /// historical size there and `s.FontSize = s.FontSize` never changes the output.
    pub fn font_size_pixels(&self, px_per_point: f32) -> f32 {
        self.font_size_points() as f32 * px_per_point
    }

    pub fn validate(&self) -> Result<()> {
        if self.string.len() > MAX_TEXT_BYTES {
            return Err("Subtitle String exceeds 4096 UTF-8 bytes".into());
        }
        if let Some(size) = self.font_size {
            validate_font_size(size)?;
        }
        if let Some(color) = self.color {
            validate_rgb(color)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct AxesExtras {
    pub texts: Vec<TextObject>,
    /// Leaf labels at integer x positions, used by dendrogram (not persisted).
    pub leaf_ticks: Option<Arc<Vec<String>>>,
    pub title_identity: ObjectIdentity,
    pub subtitle_identity: ObjectIdentity,
    pub subtitle: Subtitle,
    pub xlabel_identity: ObjectIdentity,
    pub ylabel_identity: ObjectIdentity,
    pub zlabel_identity: ObjectIdentity,
    pub title_style: LabelStyle,
    pub label_styles: [LabelStyle; 3],
    pub label_interpreters: [super::text_markup::Interpreter; 4],
    pub label_lines: [Option<super::text_markup::TextSource>; 4],
    pub tick_label_interpreter: super::text_markup::Interpreter,
    pub legend_interpreter: super::text_markup::Interpreter,
    pub legend_num_columns: Option<usize>,
    pub legend_orientation: super::LegendOrientation,
    pub legend_box: super::LegendBox,
    pub legend_style: LabelStyle,
    pub colorbar: Option<Colorbar>,
    pub colormap: Option<Arc<Vec<[f64; 3]>>>,
    pub clim: Option<[f64; 2]>,
    /// Automatic limits from mapped indexed patches when no explicit CLim is set.
    pub(crate) indexed_clim: Option<[f64; 2]>,
    pub auto_clim: Option<[f64; 2]>,
    pub font_size: Option<f64>,
    pub color_order: Option<Arc<Vec<[f64; 3]>>>,
    pub yy: Option<YyAxis>,
    /// Image objects, drawn under the series in creation order.
    pub images: Vec<super::ImageObject>,
    /// 3-D volume objects (`volshow`).
    pub volumes: Vec<super::VolumeObject>,
    /// `axis ij` / YDir 'reverse': y grows downwards.
    pub y_reversed: bool,
    /// `axis equal` / `axis image`: one data unit is as long in x as in y.
    pub equal_aspect: bool,
    /// `axis off`: no frame, ticks, tick labels or axis labels.
    pub axis_off: bool,
    pub square_plot_box: bool,
    pub tag: String,
    /// Per-axis NumericRuler settings: [X, Y, Z].
    pub rulers: [AxisRuler; 3],
    /// Process-local identities for `ax.XAxis`, `ax.YAxis`, `ax.ZAxis`.
    pub ruler_identities: [ObjectIdentity; 3],
    /// Tick direction (`None` = default `'in'` without altering legacy SVG output).
    pub tick_dir: Option<TickDir>,
    /// Axes `Layer`: grid and ticks under (default) or over the children.
    pub layer: AxesLayer,
    /// Tick length `[2D, 3D]` (`None` = default `[0.01, 0.025]`).
    pub tick_length: Option<[f64; 2]>,
    /// Axes box/line/tick width in points (`None` = default `0.5`).
    pub line_width: Option<f64>,
    /// `xline`/`yline` ConstantLine objects.
    pub constant_lines: Vec<ConstantLineObject>,
    /// `rectangle` objects.
    pub rectangles: Vec<RectangleObject>,
}
impl PartialEq for AxesExtras {
    /// Metadata comparison; object identities are compared only for yyaxis
    /// membership, never sampled data.
    fn eq(&self, other: &Self) -> bool {
        self.leaf_ticks == other.leaf_ticks
            && self.texts.len() == other.texts.len()
            && self.texts.iter().zip(&other.texts).all(|(a, b)| a.same(b))
            && self.title_style == other.title_style
            && self.label_styles == other.label_styles
            && self.subtitle == other.subtitle
            && self.label_interpreters == other.label_interpreters
            && self.label_lines == other.label_lines
            && self.tick_label_interpreter == other.tick_label_interpreter
            && self.legend_interpreter == other.legend_interpreter
            && self.legend_num_columns == other.legend_num_columns
            && self.legend_orientation == other.legend_orientation
            && self.legend_box == other.legend_box
            && self.legend_style == other.legend_style
            && match (&self.colorbar, &other.colorbar) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same(b),
                _ => false,
            }
            && self.colormap.as_deref() == other.colormap.as_deref()
            && self.clim == other.clim
            && self.indexed_clim == other.indexed_clim
            && self.auto_clim == other.auto_clim
            && self.font_size == other.font_size
            && self.color_order.as_deref() == other.color_order.as_deref()
            && match (&self.yy, &other.yy) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same(b),
                _ => false,
            }
            && self.images.len() == other.images.len()
            && self.images.iter().zip(&other.images).all(|(a, b)| {
                a.identity.same_object(&b.identity)
                    && Arc::ptr_eq(&a.data, &b.data)
                    && (a.rows, a.cols) == (b.rows, b.cols)
                    && a.mapping == b.mapping
                    && a.x == b.x
                    && a.y == b.y
                    && a.visible == b.visible
            })
            && self.volumes.len() == other.volumes.len()
            && self
                .volumes
                .iter()
                .zip(&other.volumes)
                .all(|(a, b)| a.same_metadata(b))
            && self.y_reversed == other.y_reversed
            && self.equal_aspect == other.equal_aspect
            && self.axis_off == other.axis_off
            && self.square_plot_box == other.square_plot_box
            && self.tag == other.tag
            && self.rulers == other.rulers
            && self.tick_dir == other.tick_dir
            && self.layer == other.layer
            && self.tick_length == other.tick_length
            && self.line_width == other.line_width
            && self.constant_lines.len() == other.constant_lines.len()
            && self
                .constant_lines
                .iter()
                .zip(&other.constant_lines)
                .all(|(a, b)| a.same(b))
            && self.rectangles.len() == other.rectangles.len()
            && self
                .rectangles
                .iter()
                .zip(&other.rectangles)
                .all(|(a, b)| a.same(b))
    }
}
impl AxesExtras {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
    pub fn validate(&self) -> Result<()> {
        if self.tag.len() > 4096 {
            return Err("Axes Tag exceeds 4096 bytes".into());
        }
        if self.texts.len() > MAX_TEXTS {
            return Err("Axes exceeds 256 text objects".into());
        }
        if self.images.len() > super::image::MAX_IMAGES {
            return Err("Axes exceeds 64 image objects".into());
        }
        let mut pixels = 0usize;
        for image in &self.images {
            image.validate()?;
            pixels = pixels.saturating_add(image.rows * image.cols);
        }
        if pixels > super::image::MAX_IMAGE_PIXELS {
            return Err(format!(
                "An axes holds at most {} image pixels",
                super::image::MAX_IMAGE_PIXELS
            ));
        }
        if self.volumes.len() > super::volume::MAX_VOLUMES {
            return Err(format!(
                "Axes exceeds {} volume objects",
                super::volume::MAX_VOLUMES
            ));
        }
        let mut voxels = 0usize;
        for volume in &self.volumes {
            volume.validate()?;
            let n = volume.dims[0] * volume.dims[1] * volume.dims[2];
            voxels = voxels.saturating_add(n);
        }
        if voxels > super::volume::MAX_VOLUME_VOXELS {
            return Err(format!(
                "An axes holds at most {} volume voxels",
                super::volume::MAX_VOLUME_VOXELS
            ));
        }
        for text in &self.texts {
            text.validate()?;
        }
        if self.constant_lines.len() > MAX_CONSTANT_LINES {
            return Err("Axes exceeds 256 constant line objects".into());
        }
        for line in &self.constant_lines {
            line.validate()?;
        }
        if self.rectangles.len() > MAX_RECTANGLES {
            return Err("Axes exceeds 256 rectangle objects".into());
        }
        for rect in &self.rectangles {
            rect.validate()?;
        }
        if let Some(map) = &self.colormap {
            validate_map(map, MAX_COLORMAP_ROWS, "Colormap")?;
        }
        if let Some(order) = &self.color_order {
            validate_map(order, MAX_COLOR_ORDER_ROWS, "ColorOrder")?;
        }
        if let Some(limits) = self.clim
            && !(limits[0].is_finite() && limits[1].is_finite() && limits[0] < limits[1])
        {
            return Err("CLim must be two finite increasing values".into());
        }
        if let Some(limits) = self.auto_clim
            && !(limits[0].is_finite() && limits[1].is_finite() && limits[0] < limits[1])
        {
            return Err("CLim must be two finite increasing values".into());
        }
        if let Some(size) = self.font_size {
            validate_font_size(size)?;
        }
        self.subtitle.validate()?;
        self.title_style.validate()?;
        for style in self.label_styles {
            style.validate()?;
        }
        self.legend_style.validate()?;
        for ruler in &self.rulers {
            ruler.validate()?;
        }
        if let Some(len) = self.tick_length {
            validate_tick_length(len)?;
        }
        if let Some(width) = self.line_width {
            super::validate_line_width(width)?;
        }
        if let Some(bar) = &self.colorbar {
            if bar.label.len() > MAX_TEXT_BYTES {
                return Err("Colorbar label exceeds 4096 UTF-8 bytes".into());
            }
            if bar.title.len() > MAX_TEXT_BYTES {
                return Err("Colorbar title exceeds 4096 UTF-8 bytes".into());
            }
            if let Some(size) = bar.font_size {
                validate_font_size(size)?;
            }
        }
        if let Some(yy) = &self.yy {
            validate_rgb(yy.left_color)?;
            validate_rgb(yy.right_color)?;
            if yy.right.len() > super::MAX_PLOT_SERIES {
                return Err("Right y-axis exceeds series storage budget".into());
            }
            if let Some(limits) = yy.right_ylim {
                super::validate_limits(limits)?;
            }
            if yy.right_label.len() > MAX_TEXT_BYTES {
                return Err("Right y-axis label exceeds 4096 UTF-8 bytes".into());
            }
            yy.right_label_style.validate()?;
        }
        Ok(())
    }
    pub(super) fn automatic_color_limits(&self) -> Option<[f64; 2]> {
        match (self.indexed_clim, self.auto_clim) {
            (Some(a), Some(b)) => Some([a[0].min(b[0]), a[1].max(b[1])]),
            (a, b) => a.or(b),
        }
    }
    /// Effective colour limits: explicit CLim, else automatic CLim from
    /// mapped data, else MATLAB's documented [0 1] default.
    pub fn color_limits(&self) -> [f64; 2] {
        self.clim
            .or(self.automatic_color_limits())
            .unwrap_or([0., 1.])
    }
    pub fn effective_colormap(&self) -> Arc<Vec<[f64; 3]>> {
        self.colormap
            .clone()
            .unwrap_or_else(|| Arc::new(super::colormap::default_map()))
    }
    /// Colorbar font size: an explicit colorbar FontSize as set, else 90 % of
    /// the axes font size (10 pt default). Read back and drawn alike.
    pub fn colorbar_font_size(&self) -> f64 {
        self.colorbar
            .as_ref()
            .and_then(|bar| bar.font_size)
            .unwrap_or_else(|| self.font_size.unwrap_or(10.) * 0.9)
    }
    pub fn effective_color_order(&self) -> Arc<Vec<[f64; 3]>> {
        self.color_order
            .clone()
            .unwrap_or_else(|| Arc::new(default_color_order()))
    }
    /// Identities of every object owned here (for handle lookup).
    pub fn owns(&self, id: &ObjectIdentity) -> bool {
        self.title_identity.same_object(id)
            || self.subtitle_identity.same_object(id)
            || self.xlabel_identity.same_object(id)
            || self.ylabel_identity.same_object(id)
            || self.zlabel_identity.same_object(id)
            || self.ruler_identities.iter().any(|r| r.same_object(id))
            || self.texts.iter().any(|t| t.identity.same_object(id))
            || self.images.iter().any(|i| i.identity.same_object(id))
            || self.volumes.iter().any(|v| v.identity.same_object(id))
            || self
                .constant_lines
                .iter()
                .any(|c| c.identity.same_object(id))
            || self.rectangles.iter().any(|r| r.identity.same_object(id))
            || self.colorbar.as_ref().is_some_and(|c| {
                c.identity.same_object(id)
                    || c.label_identity.same_object(id)
                    || c.title_identity.same_object(id)
            })
            || self
                .yy
                .as_ref()
                .is_some_and(|y| y.right_label_identity.same_object(id))
    }
    pub fn ruler_index(&self, id: &ObjectIdentity) -> Option<usize> {
        self.ruler_identities.iter().position(|r| r.same_object(id))
    }
}

/// The seven default line colours as normalized RGB (same bytes as `COLORS`).
pub fn default_color_order() -> Vec<[f64; 3]> {
    super::COLORS
        .iter()
        .map(|c| c.map(|v| f64::from(v) / 255.))
        .collect()
}
pub fn validate_font_size(size: f64) -> Result<()> {
    if !size.is_finite() || !(1. ..=200.).contains(&size) {
        return Err("FontSize must be a finite value in 1..200 points".into());
    }
    Ok(())
}
fn validate_rgb(rgb: [f64; 3]) -> Result<()> {
    if rgb
        .iter()
        .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
    {
        return Err("Colors must have RGB components in [0,1]".into());
    }
    Ok(())
}
fn validate_map(map: &[[f64; 3]], rows: usize, what: &str) -> Result<()> {
    if map.is_empty() || map.len() > rows {
        return Err(format!("{what} requires 1..{rows} RGB rows"));
    }
    for rgb in map {
        validate_rgb(*rgb)?;
    }
    Ok(())
}

impl Axes {
    /// (left, right) y-limits of a yyaxis axes. Each side is bounded by its
    /// own series and explicit limits; the x range is shared.
    pub fn yy_ranges(&self) -> Option<([f64; 2], [f64; 2])> {
        let yy = self.extras.yy.as_ref()?;
        if let Some([left, right]) = yy.resolved {
            return Some((left, right));
        }
        let side = |right: bool, ylim| {
            let series = self
                .series
                .iter()
                .filter(|s| yy.is_right(&s.identity) == right)
                .cloned()
                .collect();
            Axes {
                series,
                xlim: self.xlim,
                ylim,
                xscale: self.xscale,
                yscale: AxisScale::Linear,
                ..Axes::default()
            }
            .bounds()
            .1
        };
        Some((side(false, self.ylim), side(true, yy.right_ylim)))
    }
    /// A display copy that draws right-axis series in left-axis coordinates.
    /// Shares every left-series sample buffer; right series are mapped copies.
    pub fn yy_display(&self) -> Option<Axes> {
        let yy = self.extras.yy.as_ref()?;
        if yy.resolved.is_some() {
            return None;
        }
        let (left, right) = self.yy_ranges()?;
        let mut copy = self.clone();
        let scale = (left[1] - left[0]) / (right[1] - right[0]);
        for s in &mut copy.series {
            if !yy.is_right(&s.identity) || s.kind == super::PlotKind::Patch {
                continue;
            }
            let mapped: Vec<[f64; 2]> = s
                .points
                .iter()
                .map(|[x, y]| [x, left[0] + (y - right[0]) * scale])
                .collect();
            s.points = mapped.into();
        }
        copy.ylim = Some(left);
        copy.yscale = AxisScale::Linear;
        let colors = copy.colors.get_or_insert_with(Default::default);
        colors.y = Some(yy.left_color);
        if let Some(grid) = &mut colors.grid {
            grid.y_mode = super::ColorMode::Manual;
        }
        copy.extras.yy.as_mut().unwrap().resolved = Some([left, right]);
        Some(copy)
    }
}
impl super::Figure {
    /// Display copy of the active axes when it carries a right y-axis.
    pub fn yy_display_figure(&self) -> Option<super::Figure> {
        let axes = self.axes.yy_display()?;
        let mut copy = self.clone();
        copy.axes = axes;
        Some(copy)
    }
}
