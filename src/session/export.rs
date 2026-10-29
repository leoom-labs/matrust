//! Optional export preferences; malformed settings never hide recovered work.
use crate::plot::Canvas;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    #[default]
    Png,
    Jpeg,
    Svg,
    Pdf,
    Eps,
    Tiff,
    Bmp,
    Csv,
    MatlabScript,
}
impl ExportFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Svg => "SVG",
            Self::Pdf => "PDF",
            Self::Eps => "EPS (color vector)",
            Self::Tiff => "TIFF",
            Self::Bmp => "BMP",
            Self::Csv => "Plot data CSV",
            Self::MatlabScript => "MATLAB plot script",
        }
    }
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Png => &["png"],
            Self::Jpeg => &["jpg", "jpeg"],
            Self::Svg => &["svg"],
            Self::Pdf => &["pdf"],
            Self::Eps => &["eps"],
            Self::Tiff => &["tiff", "tif"],
            Self::Bmp => &["bmp"],
            Self::Csv => &["csv"],
            Self::MatlabScript => &["m"],
        }
    }
    pub fn file_name(self, id: u32) -> String {
        let separator = if self == Self::MatlabScript { "_" } else { "-" };
        format!("figure{separator}{id}.{}", self.extensions()[0])
    }
    pub fn raster(self) -> bool {
        !matches!(
            self,
            Self::Svg | Self::Pdf | Self::Eps | Self::Csv | Self::MatlabScript
        )
    }
    pub fn vector(self) -> bool {
        matches!(self, Self::Svg | Self::Pdf | Self::Eps)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExportSettings {
    pub format: ExportFormat,
    pub width: u32,
    pub height: u32,
    pub vector: Canvas,
}
impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: ExportFormat::Png,
            width: 1440,
            height: 840,
            vector: Canvas::default(),
        }
    }
}
impl ExportSettings {
    pub fn normalize(&mut self) {
        if !(64..=8192).contains(&self.width)
            || !(64..=8192).contains(&self.height)
            || self.width as u64 * self.height as u64 > 32_000_000
        {
            self.width = 1440;
            self.height = 840;
        }
        if self.vector.validate().is_err() {
            self.vector = Canvas::default();
        }
    }
}
pub(super) fn recover_settings<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<ExportSettings, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    let mut settings = serde_json::from_value::<ExportSettings>(value).unwrap_or_default();
    settings.normalize();
    Ok(settings)
}
