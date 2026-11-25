//! Non-UI data model, MATLAB classdef code generator, round-trip parser, and
//! read-only `.mlapp` reader for App Designer (Engine 9b2).
//!
//! Plain Rust state with no egui dependency; the desktop App Designer window
//! (9b3) edits this model directly as its single source of truth.

pub mod codegen;
pub mod hooks;
pub mod mlapp;
pub mod parser;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub use codegen::generate_m;
pub use hooks::{AppDesignerLaunch, Presentation, present};
pub use mlapp::{open_mlapp, read_mlapp_bytes};
pub use parser::{load_m_source, parse_generated_m};

/// Maximum MATLAB identifier length (`namelengthmax`).
pub const MAX_IDENTIFIER_LEN: usize = 63;

/// Reserved MATLAB keywords that cannot be used as identifiers.
const MATLAB_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "classdef",
    "continue",
    "else",
    "elseif",
    "end",
    "for",
    "function",
    "global",
    "if",
    "otherwise",
    "parfor",
    "persistent",
    "return",
    "spmd",
    "switch",
    "try",
    "while",
];

/// Returns `true` if `name` is a valid MATLAB identifier (ASCII letter followed
/// by up to 62 ASCII letters, digits, or underscores, and not a MATLAB keyword).
pub fn is_valid_identifier(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_IDENTIFIER_LEN {
        return false;
    }
    if !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    if !bytes[1..]
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        return false;
    }
    !MATLAB_KEYWORDS.contains(&name)
}

/// Grid track sizing for `uigridlayout` `RowHeight` / `ColumnWidth`.
#[derive(Clone, Debug, PartialEq)]
pub enum AppGridTrack {
    Pixels(f64),
    Weight(f64),
    Fit,
}

/// 1-based row or column placement in a `uigridlayout`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridSpanSpec {
    Single(usize),
    Range(usize, usize),
}

impl GridSpanSpec {
    pub fn start(self) -> usize {
        match self {
            Self::Single(n) => n,
            Self::Range(a, _) => a,
        }
    }

    pub fn end(self) -> usize {
        match self {
            Self::Single(n) => n,
            Self::Range(_, b) => b,
        }
    }

    pub fn is_valid(self) -> bool {
        match self {
            Self::Single(n) => n >= 1,
            Self::Range(a, b) => a >= 1 && b >= a,
        }
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.start() <= other.end() && other.start() <= self.end()
    }
}

impl std::fmt::Display for GridSpanSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Single(n) => write!(f, "{n}"),
            Self::Range(a, b) => write!(f, "{a}:{b}"),
        }
    }
}

/// Root `uigridlayout` specification.
#[derive(Clone, Debug, PartialEq)]
pub struct AppGridLayout {
    pub name: String,
    pub row_height: Vec<AppGridTrack>,
    pub column_width: Vec<AppGridTrack>,
}

impl AppGridLayout {
    pub fn new(name: impl Into<String>, rows: usize, cols: usize) -> Self {
        Self {
            name: name.into(),
            row_height: vec![AppGridTrack::Weight(1.0); rows.max(1)],
            column_width: vec![AppGridTrack::Weight(1.0); cols.max(1)],
        }
    }

    pub fn rows(&self) -> usize {
        self.row_height.len()
    }

    pub fn cols(&self) -> usize {
        self.column_width.len()
    }
}

/// Root `uifigure` specification.
#[derive(Clone, Debug, PartialEq)]
pub struct AppFigure {
    pub name: String,
    pub title: String,
    pub position: [f64; 4],
}

impl Default for AppFigure {
    fn default() -> Self {
        Self {
            name: "UIFigure".into(),
            title: "MATLAB App".into(),
            position: [100.0, 100.0, 640.0, 480.0],
        }
    }
}

/// All 9a component kinds supported as child components in App Designer (9b1 types).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppComponentKind {
    // Controls
    Button,
    StateButton,
    Label,
    EditField,
    NumericEditField,
    TextArea,
    DropDown,
    ListBox,
    CheckBox,
    RadioButton,
    Slider,
    Spinner,
    Switch,
    Table,
    Axes,
    // Containers
    Panel,
    TabGroup,
    Tab,
    ButtonGroup,
}

impl AppComponentKind {
    /// All supported component kinds in palette display order.
    pub const ALL: &'static [Self] = &[
        Self::Button,
        Self::StateButton,
        Self::Label,
        Self::EditField,
        Self::NumericEditField,
        Self::TextArea,
        Self::DropDown,
        Self::ListBox,
        Self::CheckBox,
        Self::RadioButton,
        Self::Slider,
        Self::Spinner,
        Self::Switch,
        Self::Table,
        Self::Axes,
        Self::Panel,
        Self::TabGroup,
        Self::Tab,
        Self::ButtonGroup,
    ];

    /// Qualified MATLAB class name used in `properties (Access = public)`.
    pub fn matlab_class_name(self) -> &'static str {
        match self {
            Self::Button => "matlab.ui.control.Button",
            Self::StateButton => "matlab.ui.control.StateButton",
            Self::Label => "matlab.ui.control.Label",
            Self::EditField => "matlab.ui.control.EditField",
            Self::NumericEditField => "matlab.ui.control.NumericEditField",
            Self::TextArea => "matlab.ui.control.TextArea",
            Self::DropDown => "matlab.ui.control.DropDown",
            Self::ListBox => "matlab.ui.control.ListBox",
            Self::CheckBox => "matlab.ui.control.CheckBox",
            Self::RadioButton => "matlab.ui.control.RadioButton",
            Self::Slider => "matlab.ui.control.Slider",
            Self::Spinner => "matlab.ui.control.Spinner",
            Self::Switch => "matlab.ui.control.Switch",
            Self::Table => "matlab.ui.control.Table",
            Self::Axes => "matlab.ui.control.UIAxes",
            Self::Panel => "matlab.ui.container.Panel",
            Self::TabGroup => "matlab.ui.container.TabGroup",
            Self::Tab => "matlab.ui.container.Tab",
            Self::ButtonGroup => "matlab.ui.container.ButtonGroup",
        }
    }

    pub fn from_matlab_class_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|k| k.matlab_class_name() == name)
    }

    /// Short human-readable label for the UI palette.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Button => "Button",
            Self::StateButton => "State Button",
            Self::Label => "Label",
            Self::EditField => "Edit Field (Text)",
            Self::NumericEditField => "Edit Field (Numeric)",
            Self::TextArea => "Text Area",
            Self::DropDown => "Drop Down",
            Self::ListBox => "List Box",
            Self::CheckBox => "Check Box",
            Self::RadioButton => "Radio Button",
            Self::Slider => "Slider",
            Self::Spinner => "Spinner",
            Self::Switch => "Switch",
            Self::Table => "Table",
            Self::Axes => "Axes",
            Self::Panel => "Panel",
            Self::TabGroup => "Tab Group",
            Self::Tab => "Tab",
            Self::ButtonGroup => "Button Group",
        }
    }

    /// Base identifier prefix used when generating a default unique component name.
    pub fn default_name_prefix(self) -> &'static str {
        match self {
            Self::Button => "Button",
            Self::StateButton => "StateButton",
            Self::Label => "Label",
            Self::EditField => "EditField",
            Self::NumericEditField => "NumericEditField",
            Self::TextArea => "TextArea",
            Self::DropDown => "DropDown",
            Self::ListBox => "ListBox",
            Self::CheckBox => "CheckBox",
            Self::RadioButton => "RadioButton",
            Self::Slider => "Slider",
            Self::Spinner => "Spinner",
            Self::Switch => "Switch",
            Self::Table => "UITable",
            Self::Axes => "UIAxes",
            Self::Panel => "Panel",
            Self::TabGroup => "TabGroup",
            Self::Tab => "Tab",
            Self::ButtonGroup => "ButtonGroup",
        }
    }

    pub fn is_container(self) -> bool {
        matches!(
            self,
            Self::Panel | Self::TabGroup | Self::Tab | Self::ButtonGroup
        )
    }

    pub fn default_position(self) -> [f64; 4] {
        match self {
            Self::Button | Self::StateButton => [100.0, 100.0, 100.0, 22.0],
            Self::Label => [100.0, 100.0, 50.0, 22.0],
            Self::EditField | Self::NumericEditField => [100.0, 100.0, 100.0, 22.0],
            Self::TextArea => [100.0, 100.0, 150.0, 60.0],
            Self::DropDown => [100.0, 100.0, 100.0, 22.0],
            Self::ListBox => [100.0, 100.0, 100.0, 74.0],
            Self::CheckBox => [100.0, 100.0, 70.0, 22.0],
            Self::RadioButton => [11.0, 58.0, 80.0, 22.0],
            Self::Slider => [100.0, 100.0, 150.0, 3.0],
            Self::Spinner => [100.0, 100.0, 100.0, 22.0],
            Self::Switch => [100.0, 100.0, 45.0, 20.0],
            Self::Table => [20.0, 20.0, 300.0, 200.0],
            Self::Axes => [20.0, 20.0, 300.0, 250.0],
            Self::Panel => [20.0, 20.0, 260.0, 220.0],
            Self::TabGroup => [20.0, 20.0, 250.0, 210.0],
            Self::Tab => [1.0, 1.0, 250.0, 185.0],
            Self::ButtonGroup => [20.0, 20.0, 123.0, 106.0],
        }
    }

    /// Callback event property names supported by this component kind.
    pub fn supported_events(self) -> &'static [&'static str] {
        match self {
            Self::Button => &["ButtonPushedFcn"],
            Self::StateButton => &["ValueChangedFcn"],
            Self::EditField
            | Self::NumericEditField
            | Self::TextArea
            | Self::Slider
            | Self::Spinner => &["ValueChangedFcn", "ValueChangingFcn"],
            Self::DropDown | Self::ListBox | Self::CheckBox | Self::Switch => &["ValueChangedFcn"],
            Self::TabGroup | Self::ButtonGroup => &["SelectionChangedFcn"],
            Self::Table => &["CellEditCallback"],
            Self::Label | Self::Panel | Self::Tab | Self::RadioButton | Self::Axes => &[],
        }
    }

    pub fn supports_text(self) -> bool {
        matches!(
            self,
            Self::Button | Self::StateButton | Self::Label | Self::CheckBox | Self::RadioButton
        )
    }

    pub fn supports_title(self) -> bool {
        matches!(self, Self::Panel | Self::Tab | Self::ButtonGroup)
    }

    pub fn supports_items(self) -> bool {
        matches!(self, Self::DropDown | Self::ListBox | Self::Switch)
    }

    pub fn supports_limits(self) -> bool {
        matches!(self, Self::NumericEditField | Self::Slider | Self::Spinner)
    }

    pub fn supports_column_name(self) -> bool {
        matches!(self, Self::Table)
    }

    pub fn value_kind(self) -> Option<AppValueKind> {
        match self {
            Self::NumericEditField | Self::Slider | Self::Spinner => Some(AppValueKind::Number),
            Self::EditField | Self::DropDown | Self::ListBox | Self::Switch => {
                Some(AppValueKind::Text)
            }
            Self::StateButton | Self::CheckBox | Self::RadioButton => Some(AppValueKind::Bool),
            Self::TextArea => Some(AppValueKind::Lines),
            Self::Button
            | Self::Label
            | Self::Table
            | Self::Axes
            | Self::Panel
            | Self::TabGroup
            | Self::Tab
            | Self::ButtonGroup => None,
        }
    }
}

/// Expected `Value` representation for a component kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppValueKind {
    Number,
    Text,
    Bool,
    Lines,
}

/// Typed `Value` property for a component.
#[derive(Clone, Debug, PartialEq)]
pub enum AppValue {
    Number(f64),
    Text(String),
    Bool(bool),
    Lines(Vec<String>),
}

impl AppValue {
    pub fn kind(&self) -> AppValueKind {
        match self {
            Self::Number(_) => AppValueKind::Number,
            Self::Text(_) => AppValueKind::Text,
            Self::Bool(_) => AppValueKind::Bool,
            Self::Lines(_) => AppValueKind::Lines,
        }
    }
}

/// Generic property value used by `set_property` / `get_property`.
#[derive(Clone, Debug, PartialEq)]
pub enum AppPropertyValue {
    Text(String),
    Number(f64),
    Bool(bool),
    Lines(Vec<String>),
    Limits([f64; 2]),
}

/// Common editable properties on a component.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppComponentProps {
    pub text: Option<String>,
    pub title: Option<String>,
    pub value: Option<AppValue>,
    pub items: Option<Vec<String>>,
    pub limits: Option<[f64; 2]>,
    pub column_name: Option<Vec<String>>,
}

/// A component in the App Designer tree.
#[derive(Clone, Debug, PartialEq)]
pub struct AppComponent {
    pub name: String,
    pub kind: AppComponentKind,
    /// Optional container parent name. `None` means the root grid if present,
    /// otherwise the root `UIFigure`.
    pub parent: Option<String>,
    /// 1-based grid row when placed in the root `uigridlayout`.
    pub grid_row: Option<GridSpanSpec>,
    /// 1-based grid column when placed in the root `uigridlayout`.
    pub grid_column: Option<GridSpanSpec>,
    /// `[left, bottom, width, height]` used when not in a grid layout.
    pub position: [f64; 4],
    pub props: AppComponentProps,
}

impl AppComponent {
    pub fn new(name: impl Into<String>, kind: AppComponentKind) -> Self {
        Self {
            name: name.into(),
            kind,
            parent: None,
            grid_row: None,
            grid_column: None,
            position: kind.default_position(),
            props: AppComponentProps::default(),
        }
    }

    pub fn in_grid(mut self, row: usize, col: usize) -> Self {
        self.grid_row = Some(GridSpanSpec::Single(row));
        self.grid_column = Some(GridSpanSpec::Single(col));
        self
    }

    pub fn in_grid_span(mut self, row: GridSpanSpec, col: GridSpanSpec) -> Self {
        self.grid_row = Some(row);
        self.grid_column = Some(col);
        self
    }

    pub fn with_position(mut self, position: [f64; 4]) -> Self {
        self.position = position;
        self
    }

    pub fn with_parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }
}

/// A component event callback method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppCallback {
    /// Target component name (or `UIFigure` name for `CloseRequestFcn`).
    pub component: String,
    /// Event property name, e.g. `"ButtonPushedFcn"`, `"ValueChangedFcn"`.
    pub event: String,
    /// Private method name in the generated class.
    pub method_name: String,
    /// Raw MATLAB code lines inside the method body.
    pub body: String,
}

/// A private property declaration on the app class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPrivateProperty {
    pub name: String,
    pub default_expr: Option<String>,
}

/// A validation issue reported by `AppModel::validate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppProblem {
    pub code: &'static str,
    pub message: String,
}

impl AppProblem {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Result of opening an app file (`.m` or `.mlapp`).
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum AppLoadResult {
    /// Editable model reloaded from a MatRust App Designer generated `.m` file.
    Editable(AppModel),
    /// Hand-written `.m` classdef or `.mlapp` opened read-only as code.
    ReadOnlyCode { code: String, reason: String },
}

/// Complete non-UI App Designer model.
#[derive(Clone, Debug, PartialEq)]
pub struct AppModel {
    pub class_name: String,
    pub figure: AppFigure,
    pub grid: Option<AppGridLayout>,
    pub components: Vec<AppComponent>,
    pub private_properties: Vec<AppPrivateProperty>,
    pub startup_fcn: Option<String>,
    pub callbacks: Vec<AppCallback>,
}

impl AppModel {
    /// Create a new app model with a default `UIFigure` and a 2x2 `GridLayout`.
    pub fn new(class_name: impl Into<String>) -> Self {
        Self {
            class_name: class_name.into(),
            figure: AppFigure::default(),
            grid: Some(AppGridLayout::new("GridLayout", 2, 2)),
            components: Vec::new(),
            private_properties: Vec::new(),
            startup_fcn: None,
            callbacks: Vec::new(),
        }
    }

    /// Create a new app model without a root grid layout (absolute `Position` placement).
    pub fn new_without_grid(class_name: impl Into<String>) -> Self {
        Self {
            class_name: class_name.into(),
            figure: AppFigure::default(),
            grid: None,
            components: Vec::new(),
            private_properties: Vec::new(),
            startup_fcn: None,
            callbacks: Vec::new(),
        }
    }

    /// All currently used property names (figure, grid, components, private properties).
    pub fn used_names(&self) -> BTreeSet<&str> {
        let mut names = BTreeSet::new();
        names.insert(self.figure.name.as_str());
        if let Some(ref g) = self.grid {
            names.insert(g.name.as_str());
        }
        for c in &self.components {
            names.insert(c.name.as_str());
        }
        for p in &self.private_properties {
            names.insert(p.name.as_str());
        }
        names
    }

    /// Generate a unique valid identifier for a new component of `kind`.
    pub fn next_component_name(&self, kind: AppComponentKind) -> String {
        let used = self.used_names();
        let base = kind.default_name_prefix();
        if !used.contains(base) {
            return base.to_string();
        }
        let mut idx = 2usize;
        loop {
            let candidate = format!("{base}{idx}");
            if !used.contains(candidate.as_str()) {
                return candidate;
            }
            idx += 1;
        }
    }

    /// Generate a default callback method name for `(component, event)`.
    pub fn default_callback_method_name(&self, component: &str, event: &str) -> String {
        let suffix = event.strip_suffix("Fcn").unwrap_or(event);
        let base = format!("{component}{suffix}");
        let mut candidate = if base.len() > MAX_IDENTIFIER_LEN {
            base[..MAX_IDENTIFIER_LEN].to_string()
        } else {
            base
        };
        let mut used: BTreeSet<&str> = self
            .callbacks
            .iter()
            .map(|c| c.method_name.as_str())
            .collect();
        used.insert("createComponents");
        used.insert("startupFcn");
        used.insert("delete");
        used.insert(self.class_name.as_str());
        if !used.contains(candidate.as_str()) {
            return candidate;
        }
        let mut idx = 2usize;
        loop {
            let num = idx.to_string();
            let prefix_len = MAX_IDENTIFIER_LEN.saturating_sub(num.len());
            let prefix = &candidate[..candidate.len().min(prefix_len)];
            let next = format!("{prefix}{num}");
            if !used.contains(next.as_str()) {
                return next;
            }
            idx += 1;
            if idx > 10_000 {
                candidate = format!("Callback{idx}");
                return candidate;
            }
        }
    }

    pub fn component(&self, name: &str) -> Option<&AppComponent> {
        self.components.iter().find(|c| c.name == name)
    }

    pub fn component_mut(&mut self, name: &str) -> Option<&mut AppComponent> {
        self.components.iter_mut().find(|c| c.name == name)
    }

    /// True if `comp` is placed directly inside the root `uigridlayout`.
    pub fn is_in_root_grid(&self, comp: &AppComponent) -> bool {
        let Some(ref grid) = self.grid else {
            return false;
        };
        match comp.parent.as_deref() {
            None => true,
            Some(p) => p == grid.name,
        }
    }

    fn check_grid_placement(
        &self,
        row: GridSpanSpec,
        col: GridSpanSpec,
        ignore_name: Option<&str>,
    ) -> Result<(), String> {
        let Some(ref grid) = self.grid else {
            return Err("App has no root uigridlayout.".into());
        };
        if !row.is_valid() || row.end() > grid.rows() {
            return Err(format!(
                "Grid row {row} is out of bounds for {} row(s).",
                grid.rows()
            ));
        }
        if !col.is_valid() || col.end() > grid.cols() {
            return Err(format!(
                "Grid column {col} is out of bounds for {} column(s).",
                grid.cols()
            ));
        }
        for other in &self.components {
            if ignore_name == Some(other.name.as_str()) || !self.is_in_root_grid(other) {
                continue;
            }
            if let (Some(orow), Some(ocol)) = (other.grid_row, other.grid_column)
                && row.overlaps(orow)
                && col.overlaps(ocol)
            {
                return Err(format!(
                    "Grid cell ({row}, {col}) is already used by {}.",
                    other.name
                ));
            }
        }
        Ok(())
    }

    /// Add a component after validating its name, parent, and placement.
    pub fn add_component(&mut self, comp: AppComponent) -> Result<(), String> {
        if !is_valid_identifier(&comp.name) {
            return Err(format!("'{}' is not a valid MATLAB identifier.", comp.name));
        }
        if self.used_names().contains(comp.name.as_str()) {
            return Err(format!("Duplicate component name '{}'.", comp.name));
        }
        if let Some(ref parent) = comp.parent {
            let parent_is_figure = parent == &self.figure.name;
            let parent_is_grid = self.grid.as_ref().is_some_and(|g| &g.name == parent);
            let parent_is_container = self
                .components
                .iter()
                .any(|c| &c.name == parent && c.kind.is_container());
            if !parent_is_figure && !parent_is_grid && !parent_is_container {
                return Err(format!("Parent container '{}' does not exist.", parent));
            }
        }
        if self.is_in_root_grid(&comp) {
            let row = comp
                .grid_row
                .ok_or_else(|| format!("Component '{}' must specify a grid row.", comp.name))?;
            let col = comp
                .grid_column
                .ok_or_else(|| format!("Component '{}' must specify a grid column.", comp.name))?;
            self.check_grid_placement(row, col, None)?;
        }
        self.components.push(comp);
        Ok(())
    }

    /// Remove a component by name. Callbacks targeting the removed component are
    /// kept so `validate()` can report orphaned callbacks (or the UI can remove them).
    pub fn remove_component(&mut self, name: &str) -> Result<AppComponent, String> {
        let idx = self
            .components
            .iter()
            .position(|c| c.name == name)
            .ok_or_else(|| format!("Component '{}' does not exist.", name))?;
        Ok(self.components.remove(idx))
    }

    /// Move a component to a new grid cell or span.
    pub fn move_component_grid(
        &mut self,
        name: &str,
        row: GridSpanSpec,
        col: GridSpanSpec,
    ) -> Result<(), String> {
        let comp = self
            .component(name)
            .ok_or_else(|| format!("Component '{}' does not exist.", name))?;
        if !self.is_in_root_grid(comp) {
            return Err(format!(
                "Component '{}' is not in the root grid layout.",
                name
            ));
        }
        self.check_grid_placement(row, col, Some(name))?;
        let comp = self.component_mut(name).unwrap();
        comp.grid_row = Some(row);
        comp.grid_column = Some(col);
        Ok(())
    }

    /// Move/resize a component using `[left, bottom, width, height]`.
    pub fn move_component_position(
        &mut self,
        name: &str,
        position: [f64; 4],
    ) -> Result<(), String> {
        if position.iter().any(|x| !x.is_finite()) || position[2] < 0.0 || position[3] < 0.0 {
            return Err("Position must be finite [left bottom width height] with non-negative width and height.".into());
        }
        let comp = self
            .component_mut(name)
            .ok_or_else(|| format!("Component '{}' does not exist.", name))?;
        comp.position = position;
        Ok(())
    }

    /// Rename a component, updating any child parent references and callbacks.
    pub fn rename_component(&mut self, old_name: &str, new_name: &str) -> Result<(), String> {
        if old_name == new_name {
            return Ok(());
        }
        if !is_valid_identifier(new_name) {
            return Err(format!("'{}' is not a valid MATLAB identifier.", new_name));
        }
        if !self.components.iter().any(|c| c.name == old_name) {
            return Err(format!("Component '{}' does not exist.", old_name));
        }
        if self.used_names().contains(new_name) {
            return Err(format!("Duplicate name '{}'.", new_name));
        }
        for c in &mut self.components {
            if c.name == old_name {
                c.name = new_name.to_string();
            }
            if c.parent.as_deref() == Some(old_name) {
                c.parent = Some(new_name.to_string());
            }
        }
        for cb in &mut self.callbacks {
            if cb.component == old_name {
                cb.component = new_name.to_string();
            }
        }
        Ok(())
    }

    /// Set a common property on a component, validating property name and type for its kind.
    pub fn set_property(
        &mut self,
        component_name: &str,
        prop_name: &str,
        value: AppPropertyValue,
    ) -> Result<(), String> {
        let comp = self
            .component_mut(component_name)
            .ok_or_else(|| format!("Component '{}' does not exist.", component_name))?;
        let kind = comp.kind;
        match prop_name {
            "Text" => {
                if !kind.supports_text() {
                    return Err(format!(
                        "Property 'Text' is not supported by {}.",
                        kind.display_name()
                    ));
                }
                let AppPropertyValue::Text(s) = value else {
                    return Err("Property 'Text' expects a text value.".into());
                };
                check_single_line_text(&s, "Text")?;
                comp.props.text = Some(s);
            }
            "Title" => {
                if !kind.supports_title() {
                    return Err(format!(
                        "Property 'Title' is not supported by {}.",
                        kind.display_name()
                    ));
                }
                let AppPropertyValue::Text(s) = value else {
                    return Err("Property 'Title' expects a text value.".into());
                };
                check_single_line_text(&s, "Title")?;
                comp.props.title = Some(s);
            }
            "Items" => {
                if !kind.supports_items() {
                    return Err(format!(
                        "Property 'Items' is not supported by {}.",
                        kind.display_name()
                    ));
                }
                let AppPropertyValue::Lines(items) = value else {
                    return Err("Property 'Items' expects a list of strings.".into());
                };
                if kind == AppComponentKind::Switch && items.len() != 2 {
                    return Err("Switch 'Items' must contain exactly 2 strings.".into());
                }
                for item in &items {
                    check_single_line_text(item, "Items")?;
                }
                comp.props.items = Some(items);
            }
            "Limits" => {
                if !kind.supports_limits() {
                    return Err(format!(
                        "Property 'Limits' is not supported by {}.",
                        kind.display_name()
                    ));
                }
                let AppPropertyValue::Limits(lim) = value else {
                    return Err("Property 'Limits' expects [min, max].".into());
                };
                if lim[0].is_nan() || lim[1].is_nan() || lim[0] >= lim[1] {
                    return Err("Limits must be [min, max] with min < max.".into());
                }
                comp.props.limits = Some(lim);
            }
            "ColumnName" => {
                if !kind.supports_column_name() {
                    return Err(format!(
                        "Property 'ColumnName' is not supported by {}.",
                        kind.display_name()
                    ));
                }
                let AppPropertyValue::Lines(cols) = value else {
                    return Err("Property 'ColumnName' expects a list of strings.".into());
                };
                for col in &cols {
                    check_single_line_text(col, "ColumnName")?;
                }
                comp.props.column_name = Some(cols);
            }
            "Value" => {
                let Some(expected) = kind.value_kind() else {
                    return Err(format!(
                        "Property 'Value' is not supported by {}.",
                        kind.display_name()
                    ));
                };
                let val = match (expected, value) {
                    (AppValueKind::Number, AppPropertyValue::Number(x)) => {
                        if !x.is_finite() {
                            return Err("Numeric Value must be finite.".into());
                        }
                        AppValue::Number(x)
                    }
                    (AppValueKind::Text, AppPropertyValue::Text(s)) => {
                        check_single_line_text(&s, "Value")?;
                        AppValue::Text(s)
                    }
                    (AppValueKind::Bool, AppPropertyValue::Bool(b)) => AppValue::Bool(b),
                    (AppValueKind::Lines, AppPropertyValue::Lines(lines)) => {
                        for line in &lines {
                            check_single_line_text(line, "Value")?;
                        }
                        AppValue::Lines(lines)
                    }
                    (AppValueKind::Lines, AppPropertyValue::Text(s)) => {
                        let lines: Vec<String> = s.lines().map(ToOwned::to_owned).collect();
                        AppValue::Lines(lines)
                    }
                    _ => {
                        return Err(format!(
                            "Wrong value type for 'Value' on {}.",
                            kind.display_name()
                        ));
                    }
                };
                comp.props.value = Some(val);
            }
            _ => {
                return Err(format!("Unsupported property '{prop_name}'."));
            }
        }
        Ok(())
    }

    /// Read a common property from a component.
    pub fn get_property(&self, component_name: &str, prop_name: &str) -> Option<AppPropertyValue> {
        let comp = self.component(component_name)?;
        match prop_name {
            "Text" => comp.props.text.clone().map(AppPropertyValue::Text),
            "Title" => comp.props.title.clone().map(AppPropertyValue::Text),
            "Items" => comp.props.items.clone().map(AppPropertyValue::Lines),
            "Limits" => comp.props.limits.map(AppPropertyValue::Limits),
            "ColumnName" => comp.props.column_name.clone().map(AppPropertyValue::Lines),
            "Value" => comp.props.value.as_ref().map(|v| match v {
                AppValue::Number(x) => AppPropertyValue::Number(*x),
                AppValue::Text(s) => AppPropertyValue::Text(s.clone()),
                AppValue::Bool(b) => AppPropertyValue::Bool(*b),
                AppValue::Lines(l) => AppPropertyValue::Lines(l.clone()),
            }),
            _ => None,
        }
    }

    /// Add a callback for `(component, event)`.
    pub fn add_callback(&mut self, cb: AppCallback) -> Result<(), String> {
        if !is_valid_identifier(&cb.method_name) {
            return Err(format!(
                "'{}' is not a valid MATLAB method identifier.",
                cb.method_name
            ));
        }
        if is_reserved_method_name(&cb.method_name, &self.class_name) {
            return Err(format!(
                "Callback method name '{}' is reserved.",
                cb.method_name
            ));
        }
        if self
            .callbacks
            .iter()
            .any(|c| c.method_name == cb.method_name)
        {
            return Err(format!(
                "Duplicate callback method name '{}'.",
                cb.method_name
            ));
        }
        codegen::check_body_markers(&cb.body, &cb.method_name)?;
        self.validate_callback_target(&cb.component, &cb.event)?;
        if self
            .callbacks
            .iter()
            .any(|c| c.component == cb.component && c.event == cb.event)
        {
            return Err(format!(
                "Component '{}' already has a callback for '{}'.",
                cb.component, cb.event
            ));
        }
        self.callbacks.push(cb);
        Ok(())
    }

    fn validate_callback_target(&self, component: &str, event: &str) -> Result<(), String> {
        if component == self.figure.name {
            if event == "CloseRequestFcn" {
                return Ok(());
            }
            return Err(format!(
                "Event '{}' is not supported on figure '{}'.",
                event, component
            ));
        }
        let comp = self
            .component(component)
            .ok_or_else(|| format!("Callback target component '{}' does not exist.", component))?;
        if !comp.kind.supported_events().contains(&event) {
            return Err(format!(
                "Event '{}' is not supported on {} '{}'.",
                event,
                comp.kind.display_name(),
                component
            ));
        }
        Ok(())
    }

    /// Rename a callback method.
    pub fn rename_callback(&mut self, old_method: &str, new_method: &str) -> Result<(), String> {
        if old_method == new_method {
            return Ok(());
        }
        if !is_valid_identifier(new_method) {
            return Err(format!(
                "'{}' is not a valid MATLAB method identifier.",
                new_method
            ));
        }
        if is_reserved_method_name(new_method, &self.class_name) {
            return Err(format!(
                "Callback method name '{}' is reserved.",
                new_method
            ));
        }
        if self.callbacks.iter().any(|c| c.method_name == new_method) {
            return Err(format!("Duplicate callback method name '{}'.", new_method));
        }
        let cb = self
            .callbacks
            .iter_mut()
            .find(|c| c.method_name == old_method)
            .ok_or_else(|| format!("Callback '{}' does not exist.", old_method))?;
        cb.method_name = new_method.to_string();
        Ok(())
    }

    /// Remove a callback by method name.
    pub fn remove_callback(&mut self, method_name: &str) -> Result<AppCallback, String> {
        let idx = self
            .callbacks
            .iter()
            .position(|c| c.method_name == method_name)
            .ok_or_else(|| format!("Callback '{}' does not exist.", method_name))?;
        Ok(self.callbacks.remove(idx))
    }

    /// Update the body of an existing callback method.
    pub fn set_callback_body(&mut self, method_name: &str, body: String) -> Result<(), String> {
        let cb = self
            .callbacks
            .iter_mut()
            .find(|c| c.method_name == method_name)
            .ok_or_else(|| format!("Callback '{}' does not exist.", method_name))?;
        codegen::check_body_markers(&body, method_name)?;
        cb.body = body;
        Ok(())
    }

    /// Validate the entire model and return all detected problems.
    pub fn validate(&self) -> Vec<AppProblem> {
        let mut problems = Vec::new();

        if !is_valid_identifier(&self.class_name) {
            problems.push(AppProblem::new(
                "bad_identifier",
                format!(
                    "App class name '{}' is not a valid MATLAB identifier.",
                    self.class_name
                ),
            ));
        }
        if !is_valid_identifier(&self.figure.name) {
            problems.push(AppProblem::new(
                "bad_identifier",
                format!(
                    "Figure name '{}' is not a valid MATLAB identifier.",
                    self.figure.name
                ),
            ));
        }
        if check_single_line_text(&self.figure.title, "Figure Name").is_err() {
            problems.push(AppProblem::new(
                "invalid_text",
                "Figure title cannot contain newline or NUL characters.",
            ));
        }

        let mut seen_names: BTreeMap<&str, usize> = BTreeMap::new();
        *seen_names.entry(self.figure.name.as_str()).or_insert(0) += 1;

        if let Some(ref grid) = self.grid {
            if !is_valid_identifier(&grid.name) {
                problems.push(AppProblem::new(
                    "bad_identifier",
                    format!(
                        "Grid layout name '{}' is not a valid MATLAB identifier.",
                        grid.name
                    ),
                ));
            }
            if grid.row_height.is_empty() || grid.column_width.is_empty() {
                problems.push(AppProblem::new(
                    "invalid_grid",
                    "Grid layout must have at least 1 row and 1 column.",
                ));
            }
            *seen_names.entry(grid.name.as_str()).or_insert(0) += 1;
        }

        for comp in &self.components {
            if !is_valid_identifier(&comp.name) {
                problems.push(AppProblem::new(
                    "bad_identifier",
                    format!(
                        "Component name '{}' is not a valid MATLAB identifier.",
                        comp.name
                    ),
                ));
            }
            *seen_names.entry(comp.name.as_str()).or_insert(0) += 1;

            if let Some(ref parent) = comp.parent {
                let exists = parent == &self.figure.name
                    || self.grid.as_ref().is_some_and(|g| &g.name == parent)
                    || self
                        .components
                        .iter()
                        .any(|c| &c.name == parent && c.kind.is_container());
                if !exists {
                    problems.push(AppProblem::new(
                        "missing_parent",
                        format!(
                            "Component '{}' references missing parent '{}'.",
                            comp.name, parent
                        ),
                    ));
                }
            }

            // Validate single-line text properties against newlines/NUL.
            if let Some(ref s) = comp.props.text
                && check_single_line_text(s, "Text").is_err()
            {
                problems.push(AppProblem::new(
                    "invalid_text",
                    format!(
                        "Component '{}' Text cannot contain newline or NUL characters.",
                        comp.name
                    ),
                ));
            }
            if let Some(ref s) = comp.props.title
                && check_single_line_text(s, "Title").is_err()
            {
                problems.push(AppProblem::new(
                    "invalid_text",
                    format!(
                        "Component '{}' Title cannot contain newline or NUL characters.",
                        comp.name
                    ),
                ));
            }
            if let Some(AppValue::Text(ref s)) = comp.props.value
                && check_single_line_text(s, "Value").is_err()
            {
                problems.push(AppProblem::new(
                    "invalid_text",
                    format!(
                        "Component '{}' Value cannot contain newline or NUL characters.",
                        comp.name
                    ),
                ));
            }
            if let Some(AppValue::Lines(ref lines)) = comp.props.value
                && lines
                    .iter()
                    .any(|l| check_single_line_text(l, "Value").is_err())
            {
                problems.push(AppProblem::new(
                    "invalid_text",
                    format!(
                        "Component '{}' Value lines cannot contain newline or NUL characters.",
                        comp.name
                    ),
                ));
            }
            if let Some(ref items) = comp.props.items
                && items
                    .iter()
                    .any(|i| check_single_line_text(i, "Items").is_err())
            {
                problems.push(AppProblem::new(
                    "invalid_text",
                    format!(
                        "Component '{}' Items cannot contain newline or NUL characters.",
                        comp.name
                    ),
                ));
            }
            if let Some(ref cols) = comp.props.column_name
                && cols
                    .iter()
                    .any(|c| check_single_line_text(c, "ColumnName").is_err())
            {
                problems.push(AppProblem::new(
                    "invalid_text",
                    format!(
                        "Component '{}' ColumnName cannot contain newline or NUL characters.",
                        comp.name
                    ),
                ));
            }
        }

        for prop in &self.private_properties {
            if !is_valid_identifier(&prop.name) {
                problems.push(AppProblem::new(
                    "bad_identifier",
                    format!(
                        "Private property '{}' is not a valid MATLAB identifier.",
                        prop.name
                    ),
                ));
            }
            *seen_names.entry(prop.name.as_str()).or_insert(0) += 1;
            if let Some(ref expr) = prop.default_expr
                && (expr.contains('\n') || expr.contains('\r') || expr.contains('\0'))
            {
                problems.push(AppProblem::new(
                    "invalid_expr",
                    format!(
                        "Private property '{}' default expression must be a single line.",
                        prop.name
                    ),
                ));
            }
        }

        for (name, count) in seen_names {
            if count > 1 {
                problems.push(AppProblem::new(
                    "duplicate_name",
                    format!("Duplicate property name '{name}' ({count} occurrences)."),
                ));
            }
        }

        // Grid cell bounds and collision checks.
        if let Some(ref grid) = self.grid {
            let mut placed: Vec<(&str, GridSpanSpec, GridSpanSpec)> = Vec::new();
            for comp in &self.components {
                if !self.is_in_root_grid(comp) {
                    continue;
                }
                match (comp.grid_row, comp.grid_column) {
                    (Some(row), Some(col)) => {
                        if !row.is_valid()
                            || row.end() > grid.rows()
                            || !col.is_valid()
                            || col.end() > grid.cols()
                        {
                            problems.push(AppProblem::new(
                                "grid_out_of_bounds",
                                format!(
                                    "Component '{}' grid cell ({row}, {col}) is outside {}x{} grid.",
                                    comp.name,
                                    grid.rows(),
                                    grid.cols()
                                ),
                            ));
                        }
                        for &(other_name, orow, ocol) in &placed {
                            if row.overlaps(orow) && col.overlaps(ocol) {
                                problems.push(AppProblem::new(
                                    "grid_cell_conflict",
                                    format!(
                                        "Components '{}' and '{}' overlap in the same grid cell.",
                                        other_name, comp.name
                                    ),
                                ));
                            }
                        }
                        placed.push((comp.name.as_str(), row, col));
                    }
                    _ => {
                        problems.push(AppProblem::new(
                            "missing_grid_cell",
                            format!(
                                "Component '{}' is in the grid layout but has no Row/Column.",
                                comp.name
                            ),
                        ));
                    }
                }
            }
        }

        // Callback checks.
        let mut seen_methods: BTreeSet<&str> = BTreeSet::new();
        let mut seen_pairs: BTreeSet<(&str, &str)> = BTreeSet::new();
        for cb in &self.callbacks {
            if !is_valid_identifier(&cb.method_name) {
                problems.push(AppProblem::new(
                    "bad_identifier",
                    format!(
                        "Callback method name '{}' is not a valid MATLAB identifier.",
                        cb.method_name
                    ),
                ));
            } else if is_reserved_method_name(&cb.method_name, &self.class_name) {
                problems.push(AppProblem::new(
                    "reserved_method_name",
                    format!("Callback method name '{}' is reserved.", cb.method_name),
                ));
            } else if !seen_methods.insert(cb.method_name.as_str()) {
                problems.push(AppProblem::new(
                    "duplicate_callback_method",
                    format!("Duplicate callback method name '{}'.", cb.method_name),
                ));
            }

            if cb.component == self.figure.name {
                if cb.event != "CloseRequestFcn" {
                    problems.push(AppProblem::new(
                        "unsupported_event",
                        format!(
                            "Callback '{}' uses unsupported event '{}' on figure '{}'.",
                            cb.method_name, cb.event, cb.component
                        ),
                    ));
                }
            } else if let Some(comp) = self.component(&cb.component) {
                if !comp.kind.supported_events().contains(&cb.event.as_str()) {
                    problems.push(AppProblem::new(
                        "unsupported_event",
                        format!(
                            "Callback '{}' uses unsupported event '{}' on {} '{}'.",
                            cb.method_name,
                            cb.event,
                            comp.kind.display_name(),
                            comp.name
                        ),
                    ));
                }
            } else {
                problems.push(AppProblem::new(
                    "missing_callback_component",
                    format!(
                        "Callback '{}' references missing component '{}'.",
                        cb.method_name, cb.component
                    ),
                ));
            }

            if !seen_pairs.insert((cb.component.as_str(), cb.event.as_str())) {
                problems.push(AppProblem::new(
                    "duplicate_callback_event",
                    format!(
                        "Multiple callbacks registered for '{}.{}'.",
                        cb.component, cb.event
                    ),
                ));
            }
        }

        let bodies = self
            .startup_fcn
            .iter()
            .map(|b| ("startupFcn", b.as_str()))
            .chain(
                self.callbacks
                    .iter()
                    .map(|c| (c.method_name.as_str(), c.body.as_str())),
            );
        for (method_name, body) in bodies {
            if let Err(msg) = codegen::check_body_markers(body, method_name) {
                problems.push(AppProblem::new("body_marker", msg));
            }
        }

        problems
    }

    /// Generate MATLAB `.m` classdef code for this model.
    pub fn generate_m(&self) -> Result<String, String> {
        codegen::generate_m(self)
    }
}

/// Open either a `.m` file (editable if generated by MatRust App Designer,
/// read-only otherwise) or a `.mlapp` file (always read-only as code).
pub fn open_app_file(path: &Path) -> Result<AppLoadResult, String> {
    let is_mlapp = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mlapp"));
    if is_mlapp {
        let code = mlapp::open_mlapp(path)?;
        return Ok(AppLoadResult::ReadOnlyCode {
            code,
            reason: "MatRust opens .mlapp files read-only as code; save apps as .m.".into(),
        });
    }
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("Cannot read '{}': {e}", path.display()))?;
    Ok(parser::load_m_source(&source))
}

pub(crate) fn check_single_line_text(s: &str, prop: &str) -> Result<(), String> {
    if s.contains('\n') || s.contains('\r') || s.contains('\0') {
        return Err(format!("{prop} cannot contain newline or NUL characters."));
    }
    Ok(())
}

fn is_reserved_method_name(name: &str, class_name: &str) -> bool {
    name == class_name || matches!(name, "createComponents" | "startupFcn" | "delete")
}
