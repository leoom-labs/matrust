use crate::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkerShape {
    Circle,
    Point,
    Plus,
    Cross,
    Asterisk,
    Square,
    Diamond,
    Up,
    Down,
    Left,
    Right,
}
impl MarkerShape {
    pub const ALL: [Self; 11] = [
        Self::Circle,
        Self::Point,
        Self::Plus,
        Self::Cross,
        Self::Asterisk,
        Self::Square,
        Self::Diamond,
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Circle => "Circle",
            Self::Point => "Point",
            Self::Plus => "Plus",
            Self::Cross => "Cross",
            Self::Asterisk => "Asterisk",
            Self::Square => "Square",
            Self::Diamond => "Diamond",
            Self::Up => "Up triangle",
            Self::Down => "Down triangle",
            Self::Left => "Left triangle",
            Self::Right => "Right triangle",
        }
    }
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Circle => "o",
            Self::Point => ".",
            Self::Plus => "+",
            Self::Cross => "x",
            Self::Asterisk => "*",
            Self::Square => "s",
            Self::Diamond => "d",
            Self::Up => "^",
            Self::Down => "v",
            Self::Left => "<",
            Self::Right => ">",
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "o" | "circle" => Ok(Self::Circle),
            "." | "point" => Ok(Self::Point),
            "+" => Ok(Self::Plus),
            "x" => Ok(Self::Cross),
            "*" => Ok(Self::Asterisk),
            "s" | "square" => Ok(Self::Square),
            "d" | "diamond" => Ok(Self::Diamond),
            "^" => Ok(Self::Up),
            "v" => Ok(Self::Down),
            "<" => Ok(Self::Left),
            ">" => Ok(Self::Right),
            _ => Err(
                "Supported markers: o . + x * s d ^ v < > (circle, point, square, diamond)".into(),
            ),
        }
    }
}
pub fn validate_marker_size(points: f64) -> Result<()> {
    if !points.is_finite() || !(0.1..=100.).contains(&points) {
        return Err("Marker size must be 0.1..100 points".into());
    }
    Ok(())
}
pub fn validate_marker_area(area: f64) -> Result<()> {
    if !area.is_finite() || area <= 0. {
        return Err("SizeData values must be positive and finite".into());
    }
    validate_marker_size(area.sqrt())
}
pub fn validate_alpha(alpha: f64, name: &str) -> Result<()> {
    if !alpha.is_finite() || !(0. ..=1.).contains(&alpha) {
        return Err(format!("{name} must be a finite scalar in 0..1"));
    }
    Ok(())
}
/// Scatter marker face/edge colour policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkerColor {
    #[default]
    Default,
    None,
    Flat,
    Auto,
    Rgb([u8; 3]),
}
/// Per-point scatter colour data when non-scalar.
#[derive(Clone, Debug, PartialEq)]
pub enum ScatterCData {
    Rgb(std::sync::Arc<[[u8; 3]]>),
    Indexed(std::sync::Arc<[f64]>),
}
impl super::Series {
    pub fn effective_marker(&self) -> MarkerShape {
        self.marker_shape.unwrap_or(if self.marker_filled {
            MarkerShape::Point
        } else {
            MarkerShape::Circle
        })
    }
    pub fn has_marker_style(&self) -> bool {
        self.marker_shape.is_some() || self.marker_size.is_some()
    }
    pub fn has_scatter_style(&self) -> bool {
        self.marker_sizes.is_some()
            || self.scatter_cdata.is_some()
            || self.marker_face_color != MarkerColor::Default
            || self.marker_edge_color != MarkerColor::Default
            || self.marker_face_alpha.is_some()
            || self.marker_edge_alpha.is_some()
    }
    pub fn marker_is_filled(&self) -> bool {
        self.marker_filled || self.effective_marker() == MarkerShape::Point
    }
    /// Nominal marker extent in points. Point glyphs use one third of this extent.
    pub fn marker_size_points(&self) -> f64 {
        self.marker_size.unwrap_or(if self.marker_shape.is_some() {
            6.
        } else if self.marker_filled {
            6.75
        } else {
            4.5
        })
    }
    pub fn marker_radius(&self) -> f32 {
        self.marker_radius_at(0)
    }
    pub fn marker_radius_at(&self, index: usize) -> f32 {
        let points = self
            .marker_sizes
            .as_ref()
            .and_then(|sizes| sizes.get(index).copied())
            .map_or_else(|| self.marker_size_points(), f64::sqrt);
        let factor = if self.effective_marker() == MarkerShape::Point {
            1. / 3.
        } else {
            1.
        };
        (points * 2. / 3. * factor) as f32
    }
    pub fn scatter_color_at(
        &self,
        index: usize,
        fallback: [u8; 3],
        map: &[[f64; 3]],
        clim: [f64; 2],
    ) -> [u8; 3] {
        match &self.scatter_cdata {
            Some(ScatterCData::Rgb(rgbs)) => rgbs.get(index).copied().unwrap_or(fallback),
            Some(ScatterCData::Indexed(values)) => values
                .get(index)
                .copied()
                .map_or(fallback, |v| super::colormap::scaled_rgb(map, clim, v)),
            None => fallback,
        }
    }
    pub fn validate_scatter(&self) -> Result<()> {
        if let Some(sizes) = &self.marker_sizes {
            if sizes.len() != self.points.len() {
                return Err("SizeData must match XData/YData length".into());
            }
            for &area in sizes.iter() {
                validate_marker_area(area)?;
            }
        }
        if let Some(cdata) = &self.scatter_cdata {
            let len = match cdata {
                ScatterCData::Rgb(rgbs) => rgbs.len(),
                ScatterCData::Indexed(vals) => vals.len(),
            };
            if len != self.points.len() {
                return Err("CData must match XData/YData length".into());
            }
        }
        if let Some(alpha) = self.marker_face_alpha {
            validate_alpha(alpha, "MarkerFaceAlpha")?;
        }
        if let Some(alpha) = self.marker_edge_alpha {
            validate_alpha(alpha, "MarkerEdgeAlpha")?;
        }
        Ok(())
    }
}
