//! Bounded document intent and immutable render colors. No UI dependency.
use super::{Figure, Series};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BaseTheme {
    #[default]
    Light,
    Dark,
}
impl BaseTheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FigureTheme {
    #[default]
    Auto,
    Light,
    Dark,
}
impl FigureTheme {
    pub fn resolve(self, host: BaseTheme) -> BaseTheme {
        match self {
            Self::Auto => host,
            Self::Light => BaseTheme::Light,
            Self::Dark => BaseTheme::Dark,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}
impl From<BaseTheme> for FigureTheme {
    fn from(value: BaseTheme) -> Self {
        match value {
            BaseTheme::Light => Self::Light,
            BaseTheme::Dark => Self::Dark,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    #[default]
    Auto,
    Manual,
}
impl ColorMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FigureAppearance {
    pub theme: FigureTheme,
    pub canvas_mode: ColorMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_palette: Option<super::PaletteVersion>,
}
impl Default for FigureAppearance {
    fn default() -> Self {
        Self {
            theme: FigureTheme::Auto,
            canvas_mode: ColorMode::Auto,
            series_palette: Some(super::PaletteVersion::MatRustModern1),
        }
    }
}

/// Fixed-sized policy resolved once per view/export. The dark tokens are the
/// established MatRust desktop policy, not a claim of MATLAB pixel parity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedAppearance {
    pub legacy: bool,
    pub theme: BaseTheme,
    pub canvas: [u8; 3],
    pub axes: [u8; 3],
    pub text: [u8; 3],
    pub shared_title: [u8; 3],
    pub axis: [u8; 3],
    pub grid: [u8; 3],
    pub legend_background: [u8; 3],
    pub legend_border: [u8; 3],
    pub disabled: [u8; 3],
    pub raster_background: [u8; 3],
}
impl Default for ResolvedAppearance {
    fn default() -> Self {
        Self::legacy([255; 3])
    }
}
impl ResolvedAppearance {
    pub fn legacy(canvas: [u8; 3]) -> Self {
        Self {
            legacy: true,
            theme: BaseTheme::Light,
            canvas,
            axes: [255; 3],
            text: [36; 3],
            shared_title: [36, 54, 75],
            axis: [119; 3],
            grid: [222; 3],
            legend_background: [255; 3],
            legend_border: [170; 3],
            disabled: [145; 3],
            raster_background: canvas,
        }
    }
    pub fn series_color(self, series: &Series) -> [u8; 3] {
        if !series.explicit_color
            && let Some(palette) = series.auto_palette
        {
            return palette.version.color(palette.slot, self.theme);
        }
        series.color
    }
}
impl Figure {
    /// Legacy absence resolves as fixed Light/manual. Native serialization and
    /// metadata/cache equality retain the actual absence distinctly.
    pub fn appearance_policy(&self) -> FigureAppearance {
        self.appearance.unwrap_or(FigureAppearance {
            theme: FigureTheme::Light,
            canvas_mode: ColorMode::Manual,
            series_palette: None,
        })
    }
    pub fn resolve_appearance(&self, host: BaseTheme) -> ResolvedAppearance {
        let Some(intent) = self.appearance else {
            return ResolvedAppearance::legacy(self.color);
        };
        let theme = intent.theme.resolve(host);
        let mut appearance = match theme {
            BaseTheme::Light => ResolvedAppearance {
                legacy: false,
                theme,
                canvas: [245; 3],
                axes: [255; 3],
                text: [36; 3],
                shared_title: [36, 54, 75],
                axis: [119; 3],
                grid: [38; 3],
                legend_background: [255; 3],
                legend_border: [170; 3],
                disabled: [145; 3],
                raster_background: [245; 3],
            },
            BaseTheme::Dark => ResolvedAppearance {
                legacy: false,
                theme,
                canvas: [30; 3],
                axes: [23; 3],
                text: [140; 3],
                shared_title: [140; 3],
                axis: [140; 3],
                grid: [255; 3],
                legend_background: [35; 3],
                legend_border: [72; 3],
                disabled: [145; 3],
                raster_background: [30; 3],
            },
        };
        if intent.canvas_mode == ColorMode::Manual {
            appearance.canvas = self.color;
        }
        appearance.legacy =
            intent.theme == FigureTheme::Light && intent.canvas_mode == ColorMode::Manual;
        if appearance.legacy {
            // Explicit Light/manual draws the opaque legacy grid, so it keeps the legacy grey.
            appearance.grid = ResolvedAppearance::legacy(appearance.canvas).grid;
        }
        appearance.raster_background = appearance.canvas;
        appearance
    }
    pub fn set_canvas_color(&mut self, color: [u8; 3]) {
        self.color = color;
        // A legacy document remains legacy when only its explicit RGB changes.
        if let Some(intent) = &mut self.appearance {
            intent.canvas_mode = ColorMode::Manual;
        }
    }
    pub fn set_figure_theme(&mut self, theme: FigureTheme) {
        self.appearance
            .get_or_insert(FigureAppearance {
                theme: FigureTheme::Light,
                canvas_mode: ColorMode::Manual,
                series_palette: None,
            })
            .theme = theme;
    }
    pub fn set_theme_mode(&mut self, mode: ColorMode, host: BaseTheme) {
        let theme = if mode == ColorMode::Auto {
            FigureTheme::Auto
        } else {
            self.resolve_appearance(host).theme.into()
        };
        self.set_figure_theme(theme);
    }
    pub fn set_canvas_mode(&mut self, mode: ColorMode, host: BaseTheme) {
        if mode == ColorMode::Manual {
            self.color = self.resolve_appearance(host).canvas;
        }
        self.appearance
            .get_or_insert(FigureAppearance {
                theme: FigureTheme::Light,
                canvas_mode: ColorMode::Manual,
                series_palette: None,
            })
            .canvas_mode = mode;
    }
}
