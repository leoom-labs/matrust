//! Fixed legend placement shared by native and exported figures.
use crate::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LegendLocation {
    North,
    South,
    East,
    West,
    #[default]
    Northeast,
    Northwest,
    Southeast,
    Southwest,
    Best,
    Northoutside,
    Southoutside,
    Eastoutside,
    Westoutside,
    Northeastoutside,
    Northwestoutside,
    Southeastoutside,
    Southwestoutside,
    Bestoutside,
}
impl LegendLocation {
    pub const INSIDE: [Self; 8] = [
        Self::Northeast,
        Self::Northwest,
        Self::Southeast,
        Self::Southwest,
        Self::North,
        Self::South,
        Self::East,
        Self::West,
    ];
    pub const ALL: [Self; 18] = [
        Self::Northeast,
        Self::Northwest,
        Self::Southeast,
        Self::Southwest,
        Self::North,
        Self::South,
        Self::East,
        Self::West,
        Self::Best,
        Self::Northeastoutside,
        Self::Northwestoutside,
        Self::Southeastoutside,
        Self::Southwestoutside,
        Self::Northoutside,
        Self::Southoutside,
        Self::Eastoutside,
        Self::Westoutside,
        Self::Bestoutside,
    ];
    pub fn parse(value: &str) -> Result<Self> {
        Self::ALL.into_iter().find(|p| p.name().eq_ignore_ascii_case(value))
            .ok_or_else(|| "Unsupported legend Location; use a compass position, its outside variant, best or bestoutside".into())
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::North => "north",
            Self::South => "south",
            Self::East => "east",
            Self::West => "west",
            Self::Northeast => "northeast",
            Self::Northwest => "northwest",
            Self::Southeast => "southeast",
            Self::Southwest => "southwest",
            Self::Best => "best",
            Self::Northoutside => "northoutside",
            Self::Southoutside => "southoutside",
            Self::Eastoutside => "eastoutside",
            Self::Westoutside => "westoutside",
            Self::Northeastoutside => "northeastoutside",
            Self::Northwestoutside => "northwestoutside",
            Self::Southeastoutside => "southeastoutside",
            Self::Southwestoutside => "southwestoutside",
            Self::Bestoutside => "bestoutside",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::North => "Top center",
            Self::South => "Bottom center",
            Self::East => "Middle right",
            Self::West => "Middle left",
            Self::Northeast => "Top right",
            Self::Northwest => "Top left",
            Self::Southeast => "Bottom right",
            Self::Southwest => "Bottom left",
            Self::Best => "Least overlap inside",
            Self::Northoutside => "Above axes",
            Self::Southoutside => "Below axes",
            Self::Eastoutside => "Outside right",
            Self::Westoutside => "Outside left",
            Self::Northeastoutside => "Outside top right",
            Self::Northwestoutside => "Outside top left",
            Self::Southeastoutside => "Outside bottom right",
            Self::Southwestoutside => "Outside bottom left",
            Self::Bestoutside => "Best outside (vertical)",
        }
    }
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn requires_extended_layout(self) -> bool {
        !Self::INSIDE.contains(&self)
    }
    pub fn is_outside(self) -> bool {
        self.outside_side().is_some()
    }
    /// Left, top, right, bottom. Vertical bestoutside is top-right.
    pub fn outside_side(self) -> Option<usize> {
        match self {
            Self::Westoutside | Self::Northwestoutside | Self::Southwestoutside => Some(0),
            Self::Northoutside => Some(1),
            Self::Eastoutside
            | Self::Northeastoutside
            | Self::Southeastoutside
            | Self::Bestoutside => Some(2),
            Self::Southoutside => Some(3),
            _ => None,
        }
    }
    pub fn fixed_anchor(self) -> Self {
        match self {
            Self::Best | Self::Northeastoutside | Self::Bestoutside => Self::Northeast,
            Self::Northoutside => Self::North,
            Self::Southoutside => Self::South,
            Self::Eastoutside => Self::East,
            Self::Westoutside => Self::West,
            Self::Northwestoutside => Self::Northwest,
            Self::Southeastoutside => Self::Southeast,
            Self::Southwestoutside => Self::Southwest,
            other => other,
        }
    }
    /// Fractions of remaining horizontal/vertical space, with a top-left origin.
    pub fn alignment(self) -> [f64; 2] {
        match self {
            Self::North => [0.5, 0.],
            Self::South => [0.5, 1.],
            Self::East => [1., 0.5],
            Self::West => [0., 0.5],
            Self::Northeast => [1., 0.],
            Self::Northwest => [0., 0.],
            Self::Southeast => [1., 1.],
            Self::Southwest => [0., 1.],
            other => other.fixed_anchor().alignment(),
        }
    }
}
