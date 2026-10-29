//! UI-independent, recoverable desktop preferences. Sizes are logical points.
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneMode {
    Editor,
    Figure,
    #[default]
    #[serde(other)]
    Split,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolstripTab {
    Plots,
    Apps,
    #[default]
    #[serde(other)]
    Home,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopTheme {
    Light,
    Dark,
    #[default]
    #[serde(other)]
    System,
}

// Malformed optional appearance must not discard unrelated pane preferences.
fn recover_theme<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<DesktopTheme>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<DesktopTheme>(value)
        .ok()
        .filter(|theme| *theme != DesktopTheme::System))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceDock {
    BelowFiles,
    #[default]
    #[serde(other)]
    Right,
}
fn recover_workspace_dock<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<WorkspaceDock>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<WorkspaceDock>(value)
        .ok()
        .filter(|dock| *dock != WorkspaceDock::Right))
}
fn recover_workspace_fraction<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f32>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_f64()
        .map(|x| x as f32)
        .filter(|x| x.is_finite() && (0.2..=0.8).contains(x)))
}

/// Explicit new desktop policy. Absence retains the legacy layout exactly.
/// These flags describe user intent, never inferred from script text at paint time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CenterLayout {
    pub command: bool,
    pub editor_revealed: bool,
}
impl Default for CenterLayout {
    fn default() -> Self {
        Self {
            command: true,
            editor_revealed: false,
        }
    }
}
fn recover_center<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<CenterLayout>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

/// Independent figure surfaces; absent/invalid preserves the single-figure layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FigureTiling {
    Two,
    Four,
}
fn recover_figure_tiling<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<FigureTiling>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

/// Display sizing only: no figure identities, bounds or geometry are serialized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FigureTileSize {
    Fit,
    Readable,
}
fn recover_figure_tile_size<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<FigureTileSize>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<FigureTileSize>(value)
        .ok()
        .filter(|size| *size != FigureTileSize::Fit))
}

/// Which existing left sidebar panel is shown. Hidden is explicit user intent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarPanel {
    Files,
    History,
    Examples,
    Hidden,
}
/// Bounded visibility only; no documents, focus tickets or worker state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelLayout {
    pub sidebar: SidebarPanel,
    pub workspace: bool,
    /// Bottom Command visibility. CenterLayout separately owns the central task.
    pub command: bool,
}
impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            sidebar: SidebarPanel::Files,
            workspace: true,
            command: true,
        }
    }
}
fn recover_panels<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PanelLayout>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<PanelLayout>(value)
        .ok()
        .filter(|panels| *panels != PanelLayout::default()))
}

/// Two existing script indices, never duplicated text, cursor or worker state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorColumns {
    pub left: usize,
    pub right: usize,
}
impl EditorColumns {
    pub fn valid_for(self, documents: usize, active: usize) -> bool {
        self.left != self.right
            && self.left < documents
            && self.right < documents
            && documents <= super::MAX_DOCUMENTS
            && (active == self.left || active == self.right)
    }
    fn bounded(self) -> bool {
        self.left != self.right
            && self.left < super::MAX_DOCUMENTS
            && self.right < super::MAX_DOCUMENTS
    }
}
fn recover_editor_columns<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<EditorColumns>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<EditorColumns>(value)
        .ok()
        .filter(|columns| columns.bounded()))
}

/// Tools that open as main-area tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTab {
    Designer,
    Model,
}

/// Open tool tabs in opening order and the one shown. No tool contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolTabs {
    pub open: [Option<ToolTab>; 2],
    pub selected: Option<ToolTab>,
}
impl ToolTabs {
    /// Opened tools are packed first, unique, and the selection is one of them.
    fn valid(self) -> bool {
        let [first, second] = self.open;
        first.is_some()
            && first != second
            && (second.is_some() || self.selected.is_none() || self.selected == first)
            && self
                .selected
                .is_none_or(|tool| self.open.contains(&Some(tool)))
    }
}
fn recover_tool_tabs<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ToolTabs>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<ToolTabs>(value)
        .ok()
        .filter(|tabs| tabs.valid()))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopLayout {
    /// Absent (including older sessions) opens no tool tabs.
    #[serde(
        default,
        deserialize_with = "recover_tool_tabs",
        skip_serializing_if = "Option::is_none"
    )]
    pub tools: Option<ToolTabs>,
    /// Absent or malformed retains one editor. Session validates document indices.
    #[serde(
        default,
        deserialize_with = "recover_editor_columns",
        skip_serializing_if = "Option::is_none"
    )]
    pub editor_columns: Option<EditorColumns>,
    /// Absent or invalid retains the historical Files/Workspace/Command defaults.
    #[serde(
        default,
        deserialize_with = "recover_panels",
        skip_serializing_if = "Option::is_none"
    )]
    pub panels: Option<PanelLayout>,
    /// Absence (including older sessions) fits every independent figure in its pane.
    #[serde(
        default,
        deserialize_with = "recover_figure_tile_size",
        skip_serializing_if = "Option::is_none"
    )]
    pub figure_tile_size: Option<FigureTileSize>,
    #[serde(
        default,
        deserialize_with = "recover_figure_tiling",
        skip_serializing_if = "Option::is_none"
    )]
    pub figure_tiling: Option<FigureTiling>,
    #[serde(
        default,
        deserialize_with = "recover_center",
        skip_serializing_if = "Option::is_none"
    )]
    pub center: Option<CenterLayout>,
    /// None follows the operating system; no UI runtime type is persisted.
    #[serde(
        default,
        deserialize_with = "recover_theme",
        skip_serializing_if = "Option::is_none"
    )]
    pub theme: Option<DesktopTheme>,
    pub toolstrip: ToolstripTab,
    pub toolstrip_collapsed: bool,
    pub mode: PaneMode,
    pub sidebar_width: Option<f32>,
    /// Width of the independently docked Workspace panel.
    pub workspace_width: Option<f32>,
    /// None retains the historical right-side Workspace layout.
    #[serde(
        default,
        deserialize_with = "recover_workspace_dock",
        skip_serializing_if = "Option::is_none"
    )]
    pub workspace_dock: Option<WorkspaceDock>,
    /// Preferred lower-pane share of the left column, independent of right width.
    #[serde(
        default,
        deserialize_with = "recover_workspace_fraction",
        skip_serializing_if = "Option::is_none"
    )]
    pub left_workspace_fraction: Option<f32>,
    pub console_height: Option<f32>,
    pub editor_fraction: Option<f32>,
    /// Width of the figure column beside the Command Window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub figure_width: Option<f32>,
    /// User scale relative to the operating system's display scale.
    pub ui_scale_percent: Option<u16>,
}
impl DesktopLayout {
    /// Visual policy for a fresh desktop or Reset layout. Default remains the
    /// legacy recovery fallback for saved sessions without a dock preference.
    pub fn fresh_desktop() -> Self {
        Self {
            center: Some(CenterLayout::default()),
            workspace_dock: Some(WorkspaceDock::BelowFiles),
            ..Self::default()
        }
    }

    pub fn normalize(&mut self) {
        self.editor_columns = self.editor_columns.filter(|columns| columns.bounded());
        self.tools = self.tools.filter(|tabs| tabs.valid());
        if self.panels == Some(PanelLayout::default()) {
            self.panels = None;
        }
        if self.figure_tile_size == Some(FigureTileSize::Fit) {
            self.figure_tile_size = None;
        }
        if self.theme == Some(DesktopTheme::System) {
            self.theme = None;
        }
        fn bounded(value: &mut Option<f32>, min: f32, max: f32) {
            if value.is_some_and(|v| !v.is_finite() || !(min..=max).contains(&v)) {
                *value = None;
            }
        }
        bounded(&mut self.sidebar_width, 170., 1200.);
        bounded(&mut self.workspace_width, 150., 1200.);
        bounded(&mut self.left_workspace_fraction, 0.2, 0.8);
        if self.workspace_dock == Some(WorkspaceDock::Right) {
            self.workspace_dock = None;
        }
        bounded(&mut self.console_height, 130., 1600.);
        bounded(&mut self.editor_fraction, 0.15, 0.85);
        bounded(&mut self.figure_width, 200., 1600.);
        if self
            .ui_scale_percent
            .is_some_and(|v| !(75..=150).contains(&v))
        {
            self.ui_scale_percent = None;
        }
    }

    pub fn ui_scale_percent(&self) -> u16 {
        self.ui_scale_percent
            .filter(|v| (75..=150).contains(v))
            .unwrap_or(100)
    }
}

// Preferences are optional. A malformed preference must not hide recovered work.
// The enclosing session reader bounds JSON bytes and serde bounds nesting depth.
pub(super) fn recover_layout<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<DesktopLayout, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let mut layout = serde_json::from_value::<DesktopLayout>(value).unwrap_or_default();
    layout.normalize();
    Ok(layout)
}
