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
        let factor = if self.effective_marker() == MarkerShape::Point {
            1. / 3.
        } else {
            1.
        };
        (self.marker_size_points() * 2. / 3. * factor) as f32
    }
}
