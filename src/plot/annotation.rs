//! Figure-owned shape and line annotations in normalized figure coordinates.
use super::{Figure, LinePattern, Result};
use serde::{Deserialize, Serialize};

pub const MAX_ANNOTATIONS: usize = 64;
pub const MAX_ANNOTATION_BYTES: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnnotationKind {
    Line,
    Arrow,
    DoubleArrow,
    TextArrow,
    Rectangle,
    Ellipse,
}

impl AnnotationKind {
    pub fn type_name(self) -> &'static str {
        match self {
            Self::Line => "lineshape",
            Self::Arrow => "arrowshape",
            Self::DoubleArrow => "doubleendarrowshape",
            Self::TextArrow => "textarrowshape",
            Self::Rectangle => "rectangleshape",
            Self::Ellipse => "ellipseshape",
        }
    }

    pub fn is_box(self) -> bool {
        matches!(self, Self::Rectangle | Self::Ellipse)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnnotationHeadStyle {
    #[default]
    VBack2,
    Plain,
    None,
}

impl AnnotationHeadStyle {
    pub fn name(self) -> &'static str {
        match self {
            Self::VBack2 => "vback2",
            Self::Plain => "plain",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Annotation {
    /// Process-local only: clone retains identity; decoding creates a new token.
    #[serde(skip)]
    pub identity: crate::ObjectIdentity,
    pub kind: AnnotationKind,
    /// For line-like annotations: `[x1, y1, x2 - x1, y2 - y1]`.
    /// For box-like annotations (`Rectangle`, `Ellipse`): `[x, y, w, h]`.
    pub position: [f64; 4],
    pub color: [u8; 3],
    pub edge_none: bool,
    pub line_width: f64,
    pub pattern: LinePattern,
    pub head_style: AnnotationHeadStyle,
    pub head_length: f64,
    pub head_width: f64,
    pub text: String,
    pub font_size: f64,
    pub face_color: Option<[u8; 3]>,
    pub face_alpha: f64,
    pub visible: bool,
}

// Appearance/recovery comparisons deliberately exclude process-local identity.
impl PartialEq for Annotation {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.position == other.position
            && self.color == other.color
            && self.edge_none == other.edge_none
            && self.line_width == other.line_width
            && self.pattern == other.pattern
            && self.head_style == other.head_style
            && self.head_length == other.head_length
            && self.head_width == other.head_width
            && self.text == other.text
            && self.font_size == other.font_size
            && self.face_color == other.face_color
            && self.face_alpha == other.face_alpha
            && self.visible == other.visible
    }
}

impl Annotation {
    pub fn new_line(kind: AnnotationKind, x: [f64; 2], y: [f64; 2]) -> Result<Self> {
        if kind.is_box() {
            return Err("Box annotation requires a 4-element [x y w h] position".into());
        }
        Self::with_position(kind, [x[0], y[0], x[1] - x[0], y[1] - y[0]])
    }

    pub fn new_box(kind: AnnotationKind, position: [f64; 4]) -> Result<Self> {
        if !kind.is_box() {
            return Err("Line annotation requires X and Y 2-element vectors".into());
        }
        Self::with_position(kind, position)
    }

    fn with_position(kind: AnnotationKind, position: [f64; 4]) -> Result<Self> {
        let result = Self {
            identity: crate::ObjectIdentity::default(),
            kind,
            position,
            color: [0; 3],
            edge_none: false,
            line_width: 0.5,
            pattern: LinePattern::Solid,
            head_style: AnnotationHeadStyle::VBack2,
            head_length: 10.0,
            head_width: 10.0,
            text: String::new(),
            font_size: 10.0,
            face_color: None,
            face_alpha: 1.0,
            visible: true,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn x_endpoints(&self) -> [f64; 2] {
        [self.position[0], self.position[0] + self.position[2]]
    }

    pub fn y_endpoints(&self) -> [f64; 2] {
        [self.position[1], self.position[1] + self.position[3]]
    }

    pub fn set_x_endpoints(&mut self, x: [f64; 2]) {
        self.position[0] = x[0];
        self.position[2] = x[1] - x[0];
    }

    pub fn set_y_endpoints(&mut self, y: [f64; 2]) {
        self.position[1] = y[0];
        self.position[3] = y[1] - y[0];
    }

    pub fn validate(&self) -> Result<()> {
        if self
            .position
            .iter()
            .any(|v| !v.is_finite() || !(-10.0..=10.0).contains(v))
        {
            return Err("Annotation coordinates must be finite normalized values".into());
        }
        if self.kind.is_box() && (self.position[2] < 0.0 || self.position[3] < 0.0) {
            return Err("Annotation width and height must be nonnegative".into());
        }
        super::validate_line_width(self.line_width)?;
        if !self.head_length.is_finite() || !(0.0..=200.0).contains(&self.head_length) {
            return Err("Annotation HeadLength must be a finite scalar in 0..200".into());
        }
        if !self.head_width.is_finite() || !(0.0..=200.0).contains(&self.head_width) {
            return Err("Annotation HeadWidth must be a finite scalar in 0..200".into());
        }
        if self.text.len() > 4096 {
            return Err("Annotation text exceeds 4096 bytes".into());
        }
        if !self.font_size.is_finite() || !(1.0..=200.0).contains(&self.font_size) {
            return Err("Annotation FontSize must be 1..200 points".into());
        }
        if !self.face_alpha.is_finite() || !(0.0..=1.0).contains(&self.face_alpha) {
            return Err("Annotation FaceAlpha must be a finite scalar in 0..1".into());
        }
        Ok(())
    }

    /// Top-left pixel rectangle `[left, top, right, bottom]` for a figure canvas.
    pub fn rectangle(&self, canvas: [f64; 4]) -> [f64; 4] {
        let [left, top, right, bottom] = canvas;
        let [x, y, w, h] = self.position;
        [
            left + x * (right - left),
            top + (1.0 - y - h) * (bottom - top),
            left + (x + w) * (right - left),
            top + (1.0 - y) * (bottom - top),
        ]
    }

    /// Pixel coordinates `(start, end)` for line-like annotations on `canvas`.
    pub fn pixel_endpoints(&self, canvas: [f64; 4]) -> ([f64; 2], [f64; 2]) {
        let [left, top, right, bottom] = canvas;
        let x = self.x_endpoints();
        let y = self.y_endpoints();
        let p0 = [
            left + x[0] * (right - left),
            top + (1.0 - y[0]) * (bottom - top),
        ];
        let p1 = [
            left + x[1] * (right - left),
            top + (1.0 - y[1]) * (bottom - top),
        ];
        (p0, p1)
    }

    /// Computes `(head_polygon, shaft_attach_point)` for an arrowhead at `tip`
    /// pointing from `from` -> `tip`. Returns `None` when `head_style` is `None`
    /// or the segment is degenerate.
    pub fn arrow_head_geometry(
        tip: [f64; 2],
        from: [f64; 2],
        style: AnnotationHeadStyle,
        head_length_pt: f64,
        head_width_pt: f64,
    ) -> Option<(Vec<[f64; 2]>, [f64; 2])> {
        if style == AnnotationHeadStyle::None || head_length_pt <= 0.0 || head_width_pt <= 0.0 {
            return None;
        }
        let dx = tip[0] - from[0];
        let dy = tip[1] - from[1];
        let dist = dx.hypot(dy);
        if dist <= 1e-6 {
            return None;
        }
        let scale = 4.0 / 3.0;
        let len = (head_length_pt * scale).min(dist);
        let half_w = head_width_pt * scale * 0.5;
        let ux = dx / dist;
        let uy = dy / dist;
        let nx = -uy;
        let ny = ux;
        let base = [tip[0] - len * ux, tip[1] - len * uy];
        let left_wing = [base[0] + half_w * nx, base[1] + half_w * ny];
        let right_wing = [base[0] - half_w * nx, base[1] - half_w * ny];
        match style {
            AnnotationHeadStyle::Plain => Some((vec![tip, left_wing, right_wing], base)),
            AnnotationHeadStyle::VBack2 => {
                let notch = [tip[0] - 0.7 * len * ux, tip[1] - 0.7 * len * uy];
                Some((vec![tip, left_wing, notch, right_wing], notch))
            }
            AnnotationHeadStyle::None => None,
        }
    }
}

pub(crate) fn validate_annotations(annotations: &[Annotation]) -> Result<()> {
    if annotations.len() > MAX_ANNOTATIONS {
        return Err("Figure exceeds 64 annotations".into());
    }
    let mut bytes = 0usize;
    for ann in annotations {
        ann.validate()?;
        bytes = bytes
            .checked_add(ann.text.len())
            .ok_or("Annotation text size overflow")?;
        if bytes > MAX_ANNOTATION_BYTES {
            return Err("Figure annotation text exceeds 16384 bytes".into());
        }
    }
    Ok(())
}

impl Figure {
    pub fn add_annotation(&mut self, ann: Annotation) -> Result<usize> {
        ann.validate()?;
        self.validate()?;
        if self.identity.same_object(&ann.identity)
            || self
                .textboxes
                .iter()
                .any(|b| b.identity.same_object(&ann.identity))
            || self
                .annotations
                .iter()
                .any(|a| a.identity.same_object(&ann.identity))
            || self.all_axes().any(|(_, a)| {
                a.identity.same_object(&ann.identity)
                    || a.series
                        .iter()
                        .any(|s| s.identity.same_object(&ann.identity))
                    || a.legend_state
                        .as_ref()
                        .is_some_and(|l| l.identity.same_object(&ann.identity))
                    || a.baselines
                        .iter()
                        .any(|b| b.identity.same_object(&ann.identity))
            })
        {
            return Err("A graphics object identity appears twice in one figure".into());
        }
        if self.annotations.len() == MAX_ANNOTATIONS
            || self.annotations.iter().map(|a| a.text.len()).sum::<usize>() + ann.text.len()
                > MAX_ANNOTATION_BYTES
        {
            return Err("Figure annotation limit exceeded".into());
        }
        let index = self.annotations.len();
        self.annotations.push(ann);
        Ok(index)
    }
}
