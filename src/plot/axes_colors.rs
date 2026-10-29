//! Fixed axes/text RGB snapshots; no MATLAB color-mode or theme inference.
use super::{Axes, ColorMode, ResolvedAppearance};
use crate::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxesColors {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend: Option<LegendColors>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<AxesGridStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xlabel: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ylabel: Option<[f64; 3]>,
}
impl AxesColors {
    fn fields(self) -> [Option<[f64; 3]>; 6] {
        [
            self.background,
            self.x,
            self.y,
            self.title,
            self.xlabel,
            self.ylabel,
        ]
    }
    pub fn validate(self) -> Result<()> {
        if let Some(legend) = self.legend {
            legend.validate()?;
        }
        for rgb in self.fields().into_iter().flatten() {
            Self::validate_rgb(rgb)?;
        }
        if let Some(grid) = self.grid {
            grid.validate()?;
            if (grid.x_mode == ColorMode::Manual && self.x.is_none())
                || (grid.y_mode == ColorMode::Manual && self.y.is_none())
            {
                return Err(
                    "Manual grid ruler modes require an explicit ruler RGB snapshot".into(),
                );
            }
        }
        Ok(())
    }
    pub fn validate_rgb(rgb: [f64; 3]) -> Result<()> {
        if rgb
            .into_iter()
            .any(|v| !v.is_finite() || !(0. ..=1.).contains(&v))
        {
            return Err("Axes/text Color requires three finite RGB values in [0,1]".into());
        }
        Ok(())
    }
    /// Quantize only at the rendering boundary. Stored normalized RGB stays exact.
    pub fn rgb8(rgb: [f64; 3]) -> [u8; 3] {
        rgb.map(|v| (v * 255.).round() as u8)
    }
}
impl PartialEq for AxesColors {
    fn eq(&self, other: &Self) -> bool {
        self.grid == other.grid
            && self.legend == other.legend
            && self
                .fields()
                .into_iter()
                .zip(other.fields())
                .all(|(a, b)| match (a, b) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.map(f64::to_bits) == b.map(f64::to_bits),
                    _ => false,
                })
    }
}
impl Eq for AxesColors {}

/// Optional public legend RGB snapshots. Absence retains the document policy.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegendColors {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<[f64; 3]>,
}
impl LegendColors {
    pub fn validate(self) -> Result<()> {
        for rgb in [self.background, self.text, self.border]
            .into_iter()
            .flatten()
        {
            AxesColors::validate_rgb(rgb)?;
        }
        Ok(())
    }
    /// Public normalized values are preserved; only absent policy tokens convert.
    pub fn resolved(self, appearance: ResolvedAppearance) -> [[f64; 3]; 3] {
        [
            self.background
                .unwrap_or_else(|| appearance.legend_background.map(|v| f64::from(v) / 255.)),
            self.text
                .unwrap_or_else(|| appearance.text.map(|v| f64::from(v) / 255.)),
            self.border
                .unwrap_or_else(|| appearance.legend_border.map(|v| f64::from(v) / 255.)),
        ]
    }
}
impl PartialEq for LegendColors {
    fn eq(&self, other: &Self) -> bool {
        [self.background, self.text, self.border]
            .into_iter()
            .zip([other.background, other.text, other.border])
            .all(|(a, b)| a.map(|v| v.map(f64::to_bits)) == b.map(|v| v.map(f64::to_bits)))
    }
}
impl Eq for LegendColors {}

/// Complete explicit major-grid state. Missing historical properties never
/// manufacture this record. Minor-grid styling remains outside the model.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxesGridStyle {
    pub color: [f64; 3],
    pub alpha: f64,
    pub x_mode: ColorMode,
    pub y_mode: ColorMode,
    pub color_mode: ColorMode,
    pub alpha_mode: ColorMode,
}
impl AxesGridStyle {
    /// Establish the current MatRust appearance explicitly before public edits.
    /// This is a product-policy snapshot, not inferred MATLAB factory values.
    pub fn from_appearance(appearance: ResolvedAppearance, modern: bool) -> Self {
        Self {
            color: appearance.grid.map(|v| f64::from(v) / 255.),
            alpha: if modern { 0.15 } else { 1. },
            x_mode: ColorMode::Auto,
            y_mode: ColorMode::Auto,
            color_mode: if modern {
                ColorMode::Auto
            } else {
                ColorMode::Manual
            },
            alpha_mode: if modern {
                ColorMode::Auto
            } else {
                ColorMode::Manual
            },
        }
    }
    pub fn validate(self) -> Result<()> {
        AxesColors::validate_rgb(self.color)?;
        if !self.alpha.is_finite() || !(0. ..=1.).contains(&self.alpha) {
            return Err("GridAlpha requires a finite value in [0,1]".into());
        }
        // Current primary documentation establishes 0.15. A historical saved
        // auto record with another value is not silently normalized.
        if self.alpha_mode == ColorMode::Auto && self.alpha != 0.15 {
            return Err("Automatic GridAlpha requires an explicit saved value of 0.15; other historical defaults are unverified".into());
        }
        Ok(())
    }
}
impl PartialEq for AxesGridStyle {
    fn eq(&self, other: &Self) -> bool {
        self.color.map(f64::to_bits) == other.color.map(f64::to_bits)
            && self.alpha.to_bits() == other.alpha.to_bits()
            && self.x_mode == other.x_mode
            && self.y_mode == other.y_mode
            && self.color_mode == other.color_mode
            && self.alpha_mode == other.alpha_mode
    }
}
impl Eq for AxesGridStyle {}

#[derive(Clone, Copy, Debug)]
pub struct ResolvedAxesColors {
    pub background: [u8; 3],
    pub x: [u8; 3],
    pub y: [u8; 3],
    pub title: [u8; 3],
    pub xlabel: [u8; 3],
    pub ylabel: [u8; 3],
    pub grid_x: [u8; 3],
    pub grid_y: [u8; 3],
    pub grid_alpha: f64,
}
impl Axes {
    pub fn legend_colors(&self) -> LegendColors {
        self.colors.and_then(|c| c.legend).unwrap_or_default()
    }
    /// A small immutable view projection, not a mutation of document intent.
    pub fn legend_appearance(&self, mut appearance: ResolvedAppearance) -> ResolvedAppearance {
        let [background, text, border] = self.legend_colors().resolved(appearance);
        appearance.legend_background = AxesColors::rgb8(background);
        appearance.text = AxesColors::rgb8(text);
        appearance.legend_border = AxesColors::rgb8(border);
        appearance
    }
    pub fn resolve_colors(&self, appearance: ResolvedAppearance) -> ResolvedAxesColors {
        let colors = self.colors.unwrap_or_default();
        let grid = colors.grid;
        let direction = |mode, ruler: Option<[f64; 3]>| {
            grid.map_or(appearance.grid, |grid| {
                let rgb = if grid.color_mode == ColorMode::Auto && mode == ColorMode::Manual {
                    ruler.unwrap_or_else(|| appearance.axis.map(|v| f64::from(v) / 255.))
                } else {
                    grid.color
                };
                AxesColors::rgb8(rgb)
            })
        };
        ResolvedAxesColors {
            background: colors.background.map_or(appearance.axes, AxesColors::rgb8),
            x: colors.x.map_or(appearance.axis, AxesColors::rgb8),
            y: colors.y.map_or(appearance.axis, AxesColors::rgb8),
            title: colors.title.map_or(appearance.text, AxesColors::rgb8),
            xlabel: colors
                .xlabel
                .or(colors.x)
                .map_or(appearance.text, AxesColors::rgb8),
            ylabel: colors
                .ylabel
                .or(colors.y)
                .map_or(appearance.text, AxesColors::rgb8),
            grid_x: direction(grid.map_or(ColorMode::Auto, |g| g.x_mode), colors.x),
            grid_y: direction(grid.map_or(ColorMode::Auto, |g| g.y_mode), colors.y),
            grid_alpha: grid.map_or(if appearance.legacy { 1. } else { 0.15 }, |g| g.alpha),
        }
    }
}
