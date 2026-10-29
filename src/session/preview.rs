//! Optional preview references; image payloads are never embedded in recovery.
use serde::{Deserialize, Deserializer, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewFormat {
    #[default]
    Png,
    Jpeg,
    Bmp,
    Tiff,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewWindow {
    /// Logical points: outer top-left and inner content size.
    pub position: [f32; 2],
    pub size: [f32; 2],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PreviewNavigation {
    pub scroll: [f32; 2],
    pub window: Option<PreviewWindow>,
}
impl PreviewNavigation {
    pub fn normalize(&mut self) {
        for offset in &mut self.scroll {
            if !offset.is_finite() || !(0.0..=32768.).contains(offset) {
                *offset = 0.;
            }
        }
        if self.window.is_some_and(|w| {
            w.position
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 100_000.)
                || w.size
                    .iter()
                    .any(|v| !v.is_finite() || !(64.0..=16384.).contains(v))
        }) {
            self.window = None;
        }
    }
}
fn recover_navigation<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<PreviewNavigation, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let mut state = serde_json::from_value::<PreviewNavigation>(value).unwrap_or_default();
    state.normalize();
    Ok(state)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewState {
    #[serde(default, deserialize_with = "recover_navigation")]
    pub navigation: PreviewNavigation,
    pub path: PathBuf,
    /// Zero-based page; zero for single-image formats.
    pub page: u32,
    pub fit: bool,
    pub zoom: f32,
    pub format: PreviewFormat,
}
impl PreviewState {
    pub fn paged(&self) -> bool {
        self.path
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| {
                ["pdf", "tif", "tiff"]
                    .iter()
                    .any(|e| x.eq_ignore_ascii_case(e))
            })
    }
    pub fn valid(&self) -> bool {
        self.path.is_absolute()
            && self.path.as_os_str().len() <= 32768
            && self
                .path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| {
                    [
                        "png", "jpg", "jpeg", "bmp", "tif", "tiff", "svg", "pdf", "eps",
                    ]
                    .iter()
                    .any(|e| x.eq_ignore_ascii_case(e))
                })
            && self.page < 1000
            && (self.paged() || self.page == 0)
            && self.zoom.is_finite()
            && (0.01..=8.).contains(&self.zoom)
    }
}
// A malformed optional preview must not prevent recovering scripts and figures.
pub(super) fn recover_preview<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PreviewState>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<PreviewState>(value)
        .ok()
        .filter(PreviewState::valid))
}

/// Settings are metadata only. Existence/version are checked off UI on explicit
/// rendering, never during checkpoint decode or automatic session restoration.
pub(super) fn valid_renderer(path: &std::path::Path) -> bool {
    path.is_absolute()
        && path.as_os_str().len() <= 32768
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}
pub(super) fn recover_renderer<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PathBuf>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<PathBuf>(value)
        .ok()
        .filter(|p| valid_renderer(p)))
}
