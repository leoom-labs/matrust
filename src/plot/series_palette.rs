//! Versioned automatic series colors. Dark is MatRust policy, not MATLAB glow.
use super::{BaseTheme, PlotKind, Series};
use crate::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PaletteVersion {
    MatRustModern1,
}
impl PaletteVersion {
    pub fn color(self, slot: u8, theme: BaseTheme) -> [u8; 3] {
        // Public records are validated at admission/decode. Keep standalone
        // rendering total even for a malformed, not-yet-admitted draft.
        let index = usize::from(slot) % 7;
        match (self, theme) {
            (Self::MatRustModern1, BaseTheme::Light) => LIGHT[index],
            (Self::MatRustModern1, BaseTheme::Dark) => DARK[index],
        }
    }
}
// Published gem palette values from MathWorks orderedcolors, rounded to RGB8.
const LIGHT: [[u8; 3]; 7] = [
    [17, 113, 190],
    [221, 84, 0],
    [237, 177, 32],
    [133, 22, 209],
    [59, 170, 50],
    [47, 190, 239],
    [209, 4, 139],
];
// Explicit MatRust Dark 1 product policy. No MATLAB glow/pixel-parity claim.
const DARK: [[u8; 3]; 7] = [
    [86, 180, 233],
    [240, 143, 89],
    [245, 205, 87],
    [190, 139, 242],
    [126, 207, 112],
    [90, 214, 245],
    [244, 128, 199],
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoPalette {
    pub version: PaletteVersion,
    pub slot: u8,
}
impl AutoPalette {
    pub fn validate(self) -> Result<()> {
        if self.slot >= 7 {
            return Err("Automatic palette slot must be 0..6".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteCycle {
    pub version: PaletteVersion,
    pub next_slot: u8,
}
impl PaletteCycle {
    pub fn new(version: PaletteVersion) -> Self {
        Self {
            version,
            next_slot: 0,
        }
    }
    pub fn validate(self) -> Result<()> {
        if self.next_slot >= 7 {
            return Err("Palette cursor must be 0..6".into());
        }
        Ok(())
    }
    pub fn reset(self) -> Self {
        Self::new(self.version)
    }
    pub(crate) fn admit(&mut self, series: &mut Series) {
        if !series.explicit_color && series.kind != PlotKind::Patch && series.auto_palette.is_none()
        {
            series.auto_palette = Some(AutoPalette {
                version: self.version,
                slot: self.next_slot,
            });
        }
        self.next_slot = (self.next_slot + 1) % 7;
    }
}
impl Series {
    pub(crate) fn validate_auto_palette(&self) -> Result<()> {
        if let Some(palette) = self.auto_palette {
            palette.validate()?;
            if self.kind == PlotKind::Patch {
                return Err(
                    "Patch colors require explicit RGB, not automatic palette metadata".into(),
                );
            }
        }
        Ok(())
    }
}
