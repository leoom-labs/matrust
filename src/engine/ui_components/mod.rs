//! UI component object model (Engine 9a1): real graphics handles for
//! `uifigure`, `uigridlayout`, `uipanel`, `uitabgroup`, `uitab`, `uibutton`,
//! `uilabel`, `uieditfield`, `uitextarea`, `uidropdown`, `uilistbox`,
//! `uicheckbox`, `uibuttongroup`, `uiradiobutton`, `uislider`, `uispinner`,
//! `uiswitch`, `uitable`, `uiwait`/`uiresume`, and test-only `mrUiFire`.
mod callbacks;
mod classic;
mod dialogs_ui;
mod props;
pub(crate) mod snapshot;

use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, FunctionHandle, GraphicsArray, GraphicsHandle, GraphicsKind, Matrix,
    ObjectIdentity, Result,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(super) const MAX_UI_COMPONENTS: usize = 4096;
pub(super) const MAX_GRID_TRACKS: usize = 256;
pub(super) const MAX_ITEMS: usize = 4096;
pub(super) const MAX_TEXT_BYTES: usize = 4096;
pub(in crate::engine) const UIAXES_FIRST_FIG_ID: u32 = 2_000_001;

pub(in crate::engine) fn is_uiaxes_fig_id(id: u32) -> bool {
    id >= UIAXES_FIRST_FIG_ID
}

/// All builtin names owned by `ui_components`.
const NAMES: &[&str] = &[
    "uifigure",
    "uigridlayout",
    "uipanel",
    "uitabgroup",
    "uitab",
    "uibutton",
    "uilabel",
    "uieditfield",
    "uitextarea",
    "uidropdown",
    "uilistbox",
    "uicheckbox",
    "uibuttongroup",
    "uiradiobutton",
    "uislider",
    "uispinner",
    "uiswitch",
    "uitable",
    "uiaxes",
    "uialert",
    "uiconfirm",
    "uiprogressdlg",
    "uicontrol",
    "gcbo",
    "gcbf",
    "guidata",
    "uiresume",
    "mrUiFire",
];

/// Names usable without parentheses (`fig = uifigure;`, `uiresume;`, `gcbo`, `gcbf`).
const BARE_NAMES: &[&str] = &[
    "uifigure",
    "uigridlayout",
    "uipanel",
    "uitabgroup",
    "uitab",
    "uibutton",
    "uilabel",
    "uieditfield",
    "uitextarea",
    "uidropdown",
    "uilistbox",
    "uicheckbox",
    "uibuttongroup",
    "uiradiobutton",
    "uislider",
    "uispinner",
    "uiswitch",
    "uitable",
    "uiaxes",
    "uicontrol",
    "gcbo",
    "gcbf",
    "uiresume",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiKind {
    Figure,
    GridLayout,
    Panel,
    TabGroup,
    Tab,
    Button,
    StateButton,
    Label,
    EditField,
    NumericEditField,
    TextArea,
    DropDown,
    ListBox,
    CheckBox,
    ButtonGroup,
    RadioButton,
    Slider,
    Spinner,
    Switch,
    Table,
    Axes,
    ProgressDialog,
}

impl UiKind {
    pub(crate) fn type_name(self) -> &'static str {
        match self {
            Self::Figure => "figure",
            Self::GridLayout => "uigridlayout",
            Self::Panel => "uipanel",
            Self::TabGroup => "uitabgroup",
            Self::Tab => "uitab",
            Self::Button => "uibutton",
            Self::StateButton => "uistatebutton",
            Self::Label => "uilabel",
            Self::EditField => "uieditfield",
            Self::NumericEditField => "uinumericeditfield",
            Self::TextArea => "uitextarea",
            Self::DropDown => "uidropdown",
            Self::ListBox => "uilistbox",
            Self::CheckBox => "uicheckbox",
            Self::ButtonGroup => "uibuttongroup",
            Self::RadioButton => "uiradiobutton",
            Self::Slider => "uislider",
            Self::Spinner => "uispinner",
            Self::Switch => "uiswitch",
            Self::Table => "uitable",
            Self::Axes => "axes",
            Self::ProgressDialog => "uiprogressdlg",
        }
    }

    pub(crate) fn class_name(self) -> &'static str {
        match self {
            Self::Figure => "Figure",
            Self::GridLayout => "GridLayout",
            Self::Panel => "Panel",
            Self::TabGroup => "TabGroup",
            Self::Tab => "Tab",
            Self::Button => "Button",
            Self::StateButton => "StateButton",
            Self::Label => "Label",
            Self::EditField => "EditField",
            Self::NumericEditField => "NumericEditField",
            Self::TextArea => "TextArea",
            Self::DropDown => "DropDown",
            Self::ListBox => "ListBox",
            Self::CheckBox => "CheckBox",
            Self::ButtonGroup => "ButtonGroup",
            Self::RadioButton => "RadioButton",
            Self::Slider => "Slider",
            Self::Spinner => "Spinner",
            Self::Switch => "Switch",
            Self::Table => "Table",
            Self::Axes => "UIAxes",
            Self::ProgressDialog => "ProgressDialog",
        }
    }

    pub(crate) fn is_container(self) -> bool {
        matches!(
            self,
            Self::Figure
                | Self::GridLayout
                | Self::Panel
                | Self::TabGroup
                | Self::Tab
                | Self::ButtonGroup
        )
    }

    fn default_position(self) -> [f64; 4] {
        match self {
            Self::Figure => [100., 100., 560., 420.],
            Self::GridLayout | Self::Tab => [1., 1., 560., 420.],
            Self::Panel => [1., 1., 260., 221.],
            Self::TabGroup => [1., 1., 250., 210.],
            Self::Button | Self::StateButton => [100., 100., 100., 22.],
            Self::Label => [100., 100., 50., 22.],
            Self::EditField | Self::NumericEditField => [100., 100., 100., 22.],
            Self::TextArea => [100., 100., 150., 60.],
            Self::DropDown => [100., 100., 100., 22.],
            Self::ListBox => [100., 100., 100., 74.],
            Self::CheckBox => [100., 100., 70., 22.],
            Self::ButtonGroup => [100., 100., 123., 106.],
            Self::RadioButton => [11., 58., 58., 22.],
            Self::Slider => [100., 100., 150., 3.],
            Self::Spinner => [100., 100., 100., 22.],
            Self::Switch => [100., 100., 45., 20.],
            Self::Table => [20., 20., 300., 200.],
            Self::Axes => [20., 20., 300., 250.],
            Self::ProgressDialog => [100., 100., 360., 120.],
        }
    }

    fn default_bg(self) -> [f64; 3] {
        match self {
            Self::EditField
            | Self::NumericEditField
            | Self::TextArea
            | Self::ListBox
            | Self::Spinner
            | Self::Table => [1., 1., 1.],
            _ => [0.94, 0.94, 0.94],
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum UiCallback {
    Handle(FunctionHandle),
    Cell(CellArray),
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum GridTrack {
    Pixels(f64),
    Weight(f64),
    Fit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridSpan {
    Single(usize),
    Range(usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiLayout {
    pub row: GridSpan,
    pub column: GridSpan,
}

impl Default for UiLayout {
    fn default() -> Self {
        Self {
            row: GridSpan::Single(1),
            column: GridSpan::Single(1),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum TableHeader {
    Numbered,
    None,
    Names(Vec<String>),
}

#[derive(Clone, Debug)]
pub(crate) enum TableColumnWidth {
    Auto,
    Tracks(Vec<GridTrack>),
}

#[derive(Clone, Debug)]
pub(crate) enum UiSpecific {
    Figure {
        name: String,
        color: [f64; 3],
        close_request_fcn: Option<UiCallback>,
        is_uifigure: bool,
    },
    GridLayout {
        row_height: Vec<GridTrack>,
        column_width: Vec<GridTrack>,
        padding: [f64; 4],
        row_spacing: f64,
        column_spacing: f64,
        scrollable: bool,
    },
    Panel {
        title: String,
        title_position: String,
        border_type: String,
    },
    TabGroup {
        selected_tab: Option<usize>,
        selection_changed_fcn: Option<UiCallback>,
        tab_location: String,
    },
    Tab {
        title: String,
    },
    Button {
        text: String,
        button_pushed_fcn: Option<UiCallback>,
    },
    StateButton {
        text: String,
        value: bool,
        value_changed_fcn: Option<UiCallback>,
    },
    Label {
        text: String,
        horizontal_alignment: String,
        vertical_alignment: String,
        word_wrap: bool,
    },
    EditField {
        value: String,
        editable: bool,
        placeholder: String,
        value_changed_fcn: Option<UiCallback>,
        value_changing_fcn: Option<UiCallback>,
    },
    NumericEditField {
        value: f64,
        limits: [f64; 2],
        value_display_format: String,
        editable: bool,
        lower_limit_inclusive: bool,
        upper_limit_inclusive: bool,
        round_fractional_values: bool,
        value_changed_fcn: Option<UiCallback>,
        value_changing_fcn: Option<UiCallback>,
    },
    TextArea {
        value: Vec<String>,
        editable: bool,
        word_wrap: bool,
        value_changed_fcn: Option<UiCallback>,
        value_changing_fcn: Option<UiCallback>,
    },
    DropDown {
        items: Vec<String>,
        items_data: Option<Vec<Value>>,
        value: Value,
        editable: bool,
        value_changed_fcn: Option<UiCallback>,
    },
    ListBox {
        items: Vec<String>,
        items_data: Option<Vec<Value>>,
        value: Value,
        multiselect: bool,
        value_changed_fcn: Option<UiCallback>,
    },
    CheckBox {
        text: String,
        value: bool,
        value_changed_fcn: Option<UiCallback>,
    },
    ButtonGroup {
        title: String,
        selected_object: Option<usize>,
        selection_changed_fcn: Option<UiCallback>,
    },
    RadioButton {
        text: String,
        value: bool,
    },
    Slider {
        limits: [f64; 2],
        value: f64,
        major_ticks: Vec<f64>,
        major_ticks_manual: bool,
        orientation: String,
        value_changed_fcn: Option<UiCallback>,
        value_changing_fcn: Option<UiCallback>,
    },
    Spinner {
        limits: [f64; 2],
        step: f64,
        value: f64,
        value_display_format: String,
        editable: bool,
        value_changed_fcn: Option<UiCallback>,
        value_changing_fcn: Option<UiCallback>,
    },
    Switch {
        style: String,
        items: [String; 2],
        items_data: Option<Vec<Value>>,
        value: Value,
        orientation: String,
        value_changed_fcn: Option<UiCallback>,
    },
    Table {
        data: Value,
        column_name: TableHeader,
        row_name: TableHeader,
        column_editable: Vec<bool>,
        column_width: TableColumnWidth,
        cell_edit_callback: Option<UiCallback>,
        cell_selection_callback: Option<UiCallback>,
    },
    Axes {
        fig_id: u32,
        fig_identity: ObjectIdentity,
    },
    ProgressDialog {
        dlg_id: u32,
        title: String,
        message: String,
        value: f64,
        indeterminate: bool,
        cancelable: bool,
        cancel_text: String,
        cancel_requested: bool,
        show_percentage: bool,
        icon: String,
        interpreter: String,
        headless: bool,
    },
}

/// Classic `uicontrol` metadata layered on top of 9a1's `UiNode`.
#[derive(Clone, Debug)]
pub(crate) struct ClassicMeta {
    pub(crate) style: String,
    pub(crate) units: String,
    pub(crate) min: f64,
    pub(crate) max: f64,
    pub(crate) value_num: f64,
    pub(crate) foreground_color: [f64; 3],
    pub(crate) horizontal_alignment: String,
    pub(crate) callback: Option<UiCallback>,
    pub(crate) string_raw: Value,
    pub(crate) enable_mode: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HandleVisibility {
    On,
    Callback,
    Off,
}

impl HandleVisibility {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Callback => "callback",
            Self::Off => "off",
        }
    }

    pub(crate) fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "on" => Ok(Self::On),
            "callback" => Ok(Self::Callback),
            "off" => Ok(Self::Off),
            _ => Err("HandleVisibility must be 'on', 'callback', or 'off'.".into()),
        }
    }

    pub(crate) fn is_visible(self, in_callback: bool) -> bool {
        match self {
            Self::On => true,
            Self::Callback => in_callback,
            Self::Off => false,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct UiNode {
    pub(crate) identity: ObjectIdentity,
    pub(crate) kind: UiKind,
    pub(crate) parent: Option<usize>,
    pub(crate) classic_parent: Option<ObjectIdentity>,
    pub(crate) children: Vec<usize>,
    pub(crate) tag: String,
    pub(crate) user_data: Option<Value>,
    pub(crate) enable: bool,
    pub(crate) visible: bool,
    pub(crate) handle_visibility: HandleVisibility,
    pub(crate) position: [f64; 4],
    pub(crate) layout: Option<UiLayout>,
    pub(crate) font_size: f64,
    pub(crate) background_color: [f64; 3],
    pub(crate) tooltip: String,
    pub(crate) classic: Option<ClassicMeta>,
    pub(crate) specific: UiSpecific,
}

impl UiNode {
    fn new(kind: UiKind, parent: Option<usize>, specific: UiSpecific) -> Self {
        Self {
            identity: ObjectIdentity::default(),
            kind,
            parent,
            classic_parent: None,
            children: Vec::new(),
            tag: String::new(),
            user_data: None,
            enable: true,
            visible: true,
            handle_visibility: if kind == UiKind::Figure {
                HandleVisibility::Off
            } else {
                HandleVisibility::On
            },
            position: kind.default_position(),
            layout: None,
            font_size: 12.0,
            background_color: kind.default_bg(),
            tooltip: String::new(),
            classic: None,
            specific,
        }
    }

    pub(crate) fn handle(&self) -> GraphicsHandle {
        if let UiSpecific::Axes { fig_identity, .. } = &self.specific {
            GraphicsHandle::axes(fig_identity, &self.identity)
        } else {
            GraphicsHandle::ui_component(&self.identity)
        }
    }
}

/// Process-local store of live UI components for one [`Engine`].
#[derive(Clone, Debug, Default)]
pub(crate) struct UiState {
    pub(crate) nodes: BTreeMap<usize, UiNode>,
    pub(crate) figures: Vec<usize>,
    pub(crate) waiting: BTreeSet<usize>,
    /// Backing plot `Figure` for each live `uiaxes`, keyed by `fig_id >= UIAXES_FIRST_FIG_ID`.
    pub(crate) uiaxes_figures: BTreeMap<u32, crate::plot::Figure>,
    /// The classic current figure held aside while a call draws into a uiaxes
    /// (`Engine::call_on_axes`), so chart-record pruning still sees it.
    pub(crate) held_classic: Option<(u32, crate::plot::Figure)>,
    pub(crate) next_uiaxes_fig_id: u32,
    /// `uicontrol` / `uiaxes` / `uiprogressdlg` children attached to a classic plot `Figure`,
    /// keyed by `figure.identity.recovery_key()`.
    pub(crate) classic_fig_children: BTreeMap<usize, Vec<usize>>,
    /// Per-figure `guidata` storage, keyed by the figure's `ObjectIdentity::recovery_key()`.
    pub(crate) guidata: BTreeMap<usize, Value>,
    /// Active callback object stack for `gcbo` / `gcbf`.
    pub(crate) callback_stack: Vec<GraphicsHandle>,
    /// Next unique identifier for non-blocking `uialert` sheets.
    pub(crate) next_alert_id: u32,
    /// Pending `CloseFcn` callbacks for non-blocking desktop `uialert` sheets.
    pub(crate) pending_alert_close: BTreeMap<u32, (GraphicsHandle, UiCallback)>,
    /// Parent figure key for each active non-blocking desktop `uialert` sheet (`id -> fig_key`).
    pub(crate) pending_alert_figs: BTreeMap<u32, usize>,
    /// Active blocking `uiconfirm` sheet counts per parent figure key (`fig_key -> count`).
    pub(crate) active_confirm_figs: BTreeMap<usize, usize>,
    /// Apps registered via `matlab.apps.AppBase.registerApp(app, fig)`, keyed by
    /// the UIFigure's `ObjectIdentity::recovery_key()`. Keeps the app handle alive
    /// across `clear app` in `nargout == 0` constructors and links figure deletion to app deletion.
    pub(crate) registered_apps: BTreeMap<usize, Array>,
}

impl UiState {
    pub(crate) fn roots(&self) -> impl Iterator<Item = &Array> {
        self.registered_apps.values()
    }
    pub(crate) fn is_live(&self, id: &ObjectIdentity) -> bool {
        self.nodes.contains_key(&id.recovery_key())
    }

    pub(crate) fn is_uiaxes(&self, id: &ObjectIdentity) -> bool {
        self.nodes
            .get(&id.recovery_key())
            .is_some_and(|n| n.kind == UiKind::Axes)
    }

    #[allow(dead_code)]
    pub(crate) fn is_uifigure(&self, id: &ObjectIdentity) -> bool {
        matches!(
            self.nodes.get(&id.recovery_key()).map(|n| &n.specific),
            Some(UiSpecific::Figure {
                is_uifigure: true,
                ..
            })
        )
    }

    pub(crate) fn get(&self, id: &ObjectIdentity) -> Option<&UiNode> {
        self.nodes.get(&id.recovery_key())
    }

    pub(crate) fn get_key(&self, key: usize) -> Option<&UiNode> {
        self.nodes.get(&key)
    }

    pub(crate) fn get_mut_key(&mut self, key: usize) -> Option<&mut UiNode> {
        self.nodes.get_mut(&key)
    }

    pub(crate) fn has_classic_ui(&self, fig_key: usize) -> bool {
        self.classic_fig_children
            .get(&fig_key)
            .is_some_and(|c| !c.is_empty())
            || self.pending_alert_figs.values().any(|&fk| fk == fig_key)
            || self.active_confirm_figs.contains_key(&fig_key)
    }

    pub(crate) fn clear_pending_alerts_for_figure(&mut self, fig_key: usize) {
        let mut stale_ids = Vec::new();
        self.pending_alert_figs.retain(|&id, &mut fk| {
            if fk == fig_key {
                stale_ids.push(id);
                false
            } else {
                true
            }
        });
        for id in stale_ids {
            self.pending_alert_close.remove(&id);
        }
        self.active_confirm_figs.remove(&fig_key);
    }

    pub(crate) fn delete_classic_figure_children(&mut self, fig_key: usize) {
        self.guidata.remove(&fig_key);
        self.clear_pending_alerts_for_figure(fig_key);
        if let Some(children) = self.classic_fig_children.remove(&fig_key) {
            for child_key in children {
                self.delete_subtree(child_key);
            }
        }
    }

    /// Deletes the uicontrols and ui components of a classic figure (clf);
    /// the figure's guidata stays.
    pub(crate) fn delete_classic_figure_components(&mut self, fig_key: usize) {
        if let Some(children) = self.classic_fig_children.remove(&fig_key) {
            for child_key in children {
                self.delete_subtree(child_key);
            }
        }
    }

    fn insert_node(&mut self, mut node: UiNode) -> Result<usize> {
        if self.nodes.len() >= MAX_UI_COMPONENTS {
            return Err(format!(
                "UI component limit ({MAX_UI_COMPONENTS}) exceeded; delete unused components"
            ));
        }
        let key = node.identity.recovery_key();
        let parent_key = node.parent;
        if let Some(pk) = parent_key {
            let parent = self
                .nodes
                .get(&pk)
                .ok_or("Invalid or deleted parent object.")?;
            if parent.kind == UiKind::GridLayout {
                let (rows, cols) = match &parent.specific {
                    UiSpecific::GridLayout {
                        row_height,
                        column_width,
                        ..
                    } => (row_height.len().max(1), column_width.len().max(1)),
                    _ => (1, 1),
                };
                let idx = parent.children.len();
                let r = (idx / cols).min(rows - 1) + 1;
                let c = (idx % cols) + 1;
                node.layout = Some(UiLayout {
                    row: GridSpan::Single(r),
                    column: GridSpan::Single(c),
                });
            }
        }
        let kind = node.kind;
        self.nodes.insert(key, node);
        if let Some(pk) = parent_key {
            if let Some(parent) = self.nodes.get_mut(&pk) {
                parent.children.push(key);
                match (&mut parent.specific, kind) {
                    (UiSpecific::TabGroup { selected_tab, .. }, UiKind::Tab)
                        if selected_tab.is_none() =>
                    {
                        *selected_tab = Some(key);
                    }
                    (
                        UiSpecific::ButtonGroup {
                            selected_object, ..
                        },
                        UiKind::RadioButton,
                    ) if selected_object.is_none() => {
                        *selected_object = Some(key);
                        if let Some(rb) = self.nodes.get_mut(&key)
                            && let UiSpecific::RadioButton { value, .. } = &mut rb.specific
                        {
                            *value = true;
                        }
                    }
                    _ => {}
                }
            }
        } else if kind == UiKind::Figure {
            self.figures.push(key);
        }
        Ok(key)
    }

    pub(crate) fn delete_subtree(&mut self, key: usize) -> Vec<Array> {
        let Some(node) = self.nodes.get(&key) else {
            return Vec::new();
        };
        let parent_key = node.parent;
        let classic_parent_key = node.classic_parent.as_ref().map(|id| id.recovery_key());
        // Collect entire subtree iteratively (bounded by nodes.len()).
        let mut stack = vec![key];
        let mut to_remove = Vec::new();
        while let Some(curr) = stack.pop() {
            if let Some(n) = self.nodes.get(&curr) {
                to_remove.push(curr);
                for &ch in &n.children {
                    stack.push(ch);
                }
            }
        }
        let mut detached_apps = Vec::new();
        for rem in to_remove {
            if let Some(app) = self.registered_apps.remove(&rem) {
                detached_apps.push(app);
            }
            if let Some(removed) = self.nodes.remove(&rem) {
                if let UiSpecific::Axes { fig_id, .. } = removed.specific {
                    self.uiaxes_figures.remove(&fig_id);
                }
                if removed.kind == UiKind::Figure {
                    self.guidata.remove(&rem);
                    self.clear_pending_alerts_for_figure(rem);
                }
            }
            self.waiting.remove(&rem);
            self.figures.retain(|&f| f != rem);
        }
        if let Some(cfk) = classic_parent_key
            && let Some(list) = self.classic_fig_children.get_mut(&cfk)
        {
            list.retain(|&c| c != key);
        }
        if let Some(pk) = parent_key
            && let Some(parent) = self.nodes.get_mut(&pk)
        {
            parent.children.retain(|&c| c != key);
            match &mut parent.specific {
                UiSpecific::TabGroup { selected_tab, .. } if *selected_tab == Some(key) => {
                    *selected_tab = parent.children.first().copied();
                }
                UiSpecific::ButtonGroup {
                    selected_object, ..
                } if *selected_object == Some(key) => {
                    let next = parent.children.first().copied();
                    *selected_object = next;
                    if let Some(nk) = next
                        && let Some(rb) = self.nodes.get_mut(&nk)
                        && let UiSpecific::RadioButton { value, .. } = &mut rb.specific
                    {
                        *value = true;
                    }
                }
                _ => {}
            }
        }
        detached_apps
    }
}

/// Resolve an App Designer MATLAB UI class name (`matlab.ui.Figure`,
/// `matlab.ui.container.*`, `matlab.ui.control.*`) to its canonical static name
/// and `UiKind`.
pub(in crate::engine) fn ui_kind_for_class_name(name: &str) -> Option<(&'static str, UiKind)> {
    match name {
        "matlab.ui.Figure" => Some(("matlab.ui.Figure", UiKind::Figure)),
        "matlab.ui.container.GridLayout" => {
            Some(("matlab.ui.container.GridLayout", UiKind::GridLayout))
        }
        "matlab.ui.container.Panel" => Some(("matlab.ui.container.Panel", UiKind::Panel)),
        "matlab.ui.container.TabGroup" => Some(("matlab.ui.container.TabGroup", UiKind::TabGroup)),
        "matlab.ui.container.Tab" => Some(("matlab.ui.container.Tab", UiKind::Tab)),
        "matlab.ui.container.ButtonGroup" => {
            Some(("matlab.ui.container.ButtonGroup", UiKind::ButtonGroup))
        }
        "matlab.ui.control.Button" => Some(("matlab.ui.control.Button", UiKind::Button)),
        "matlab.ui.control.StateButton" => {
            Some(("matlab.ui.control.StateButton", UiKind::StateButton))
        }
        "matlab.ui.control.Label" => Some(("matlab.ui.control.Label", UiKind::Label)),
        "matlab.ui.control.EditField" => Some(("matlab.ui.control.EditField", UiKind::EditField)),
        "matlab.ui.control.NumericEditField" => Some((
            "matlab.ui.control.NumericEditField",
            UiKind::NumericEditField,
        )),
        "matlab.ui.control.TextArea" => Some(("matlab.ui.control.TextArea", UiKind::TextArea)),
        "matlab.ui.control.DropDown" => Some(("matlab.ui.control.DropDown", UiKind::DropDown)),
        "matlab.ui.control.ListBox" => Some(("matlab.ui.control.ListBox", UiKind::ListBox)),
        "matlab.ui.control.CheckBox" => Some(("matlab.ui.control.CheckBox", UiKind::CheckBox)),
        "matlab.ui.control.RadioButton" => {
            Some(("matlab.ui.control.RadioButton", UiKind::RadioButton))
        }
        "matlab.ui.control.Slider" => Some(("matlab.ui.control.Slider", UiKind::Slider)),
        "matlab.ui.control.Spinner" => Some(("matlab.ui.control.Spinner", UiKind::Spinner)),
        "matlab.ui.control.Switch" => Some(("matlab.ui.control.Switch", UiKind::Switch)),
        "matlab.ui.control.Table" => Some(("matlab.ui.control.Table", UiKind::Table)),
        "matlab.ui.control.UIAxes" => Some(("matlab.ui.control.UIAxes", UiKind::Axes)),
        _ => None,
    }
}

/// Check `isa(handles, wanted)` when `wanted` is one of the 21 App Designer UI class names.
pub(in crate::engine) fn graphics_isa_ui_class(
    e: &Engine,
    handles: &GraphicsArray,
    wanted: &str,
) -> Option<bool> {
    let (canonical, expected_kind) = ui_kind_for_class_name(wanted)?;
    if handles.is_empty() {
        return Some(handles.empty_class() == Some(canonical));
    }
    for i in 0..handles.len() {
        let Ok(h) = handles.get_linear(i) else {
            return Some(false);
        };
        let matches = match expected_kind {
            UiKind::Axes => h.kind() == GraphicsKind::Axes && e.ui.is_uiaxes(&h.object),
            other => {
                h.kind() == GraphicsKind::UiComponent
                    && e.ui
                        .get(&h.object)
                        .is_some_and(|node| node.kind == other && node.classic.is_none())
            }
        };
        if !matches {
            return Some(false);
        }
    }
    Some(true)
}

/// Register an AppBase instance with its UIFigure so deleting/closing the figure
/// deletes the app and vice versa, and `clear app` keeps the app alive.
pub(in crate::engine) fn register_app(e: &mut Engine, app: Array, fig: &Value) -> Result<()> {
    let Value::Container(Array::Graphics(handles)) = fig else {
        return Err("registerApp requires a UI figure handle as the second argument.".into());
    };
    let h = handles.scalar_handle()?;
    if h.kind() != GraphicsKind::UiComponent {
        return Err("registerApp requires a UI figure handle as the second argument.".into());
    }
    let key = h.object.recovery_key();
    let Some(node) = e.ui.get_key(key) else {
        return Err("Invalid or deleted object.".into());
    };
    if node.kind != UiKind::Figure {
        return Err("registerApp requires a UI figure handle as the second argument.".into());
    }
    e.ui.registered_apps.insert(key, app);
    Ok(())
}

/// Delete a UI subtree and any AppBase instances registered to figures in that subtree.
pub(in crate::engine) fn delete_ui_subtree(
    e: &mut Engine,
    key: usize,
    out: &mut Execution,
) -> Result<()> {
    let detached = e.ui.delete_subtree(key);
    for app in detached {
        e.cls_delete(&Value::Container(app), out)?;
    }
    Ok(())
}

pub(in crate::engine) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

pub(in crate::engine) fn bare(name: &str) -> bool {
    BARE_NAMES.contains(&name)
}

pub(in crate::engine) fn is_live(e: &Engine, id: &ObjectIdentity) -> bool {
    e.ui.is_live(id)
}

pub(in crate::engine) fn is_uiaxes_ui_property(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "parent"
            | "layout"
            | "position"
            | "innerposition"
            | "outerposition"
            | "tag"
            | "userdata"
            | "handlevisibility"
    )
}

pub(in crate::engine) fn class_name(e: &Engine, h: &GraphicsHandle) -> &'static str {
    e.ui.get(&h.object).map_or("Graphics", |n| {
        if n.classic.is_some() {
            "UIControl"
        } else {
            n.kind.class_name()
        }
    })
}

pub(in crate::engine) fn is_property(e: &Engine, h: &GraphicsHandle, name: &str) -> bool {
    let Some(node) = e.ui.get(&h.object) else {
        return false;
    };
    if node.classic.is_some() {
        return classic::is_classic_property(&name.to_ascii_lowercase());
    }
    props::is_known_property(node.kind, name)
}

pub(in crate::engine) fn get_property(
    e: &mut Engine,
    h: &GraphicsHandle,
    args: &[Value],
) -> Result<Value> {
    props::get_property(e, h, args)
}

pub(in crate::engine) fn set_properties(
    e: &mut Engine,
    h: &GraphicsHandle,
    args: &[Value],
) -> Result<()> {
    props::set_properties(e, h, args)
}

pub(in crate::engine) fn delete_handles(
    e: &mut Engine,
    handles: &GraphicsArray,
    out: &mut Execution,
) -> Result<()> {
    for i in 0..handles.len() {
        e.check_cancel()?;
        let h = handles.get_linear(i)?;
        if h.kind() == GraphicsKind::UiComponent {
            let key = h.object.recovery_key();
            let was_progress = matches!(
                e.ui.get_key(key).map(|n| &n.specific),
                Some(UiSpecific::ProgressDialog {
                    headless: false,
                    ..
                })
            );
            delete_ui_subtree(e, key, out)?;
            if was_progress {
                e.gobj_flush_frame();
            }
        }
    }
    Ok(())
}

pub(in crate::engine) fn close_figure(
    e: &mut Engine,
    h: &GraphicsHandle,
    force: bool,
    out: &mut Execution,
) -> Result<()> {
    let key = h.object.recovery_key();
    let Some(node) = e.ui.get_key(key) else {
        return Err("Invalid or deleted object.".into());
    };
    if node.kind == UiKind::ProgressDialog {
        let (dlg_id, headless) = match &node.specific {
            UiSpecific::ProgressDialog {
                dlg_id, headless, ..
            } => (*dlg_id, *headless),
            _ => unreachable!(),
        };
        delete_ui_subtree(e, key, out)?;
        dialogs_ui::close_progress_host(e, dlg_id, headless, out)?;
        return Ok(());
    }
    if node.kind != UiKind::Figure {
        return Err("close requires one figure handle".into());
    }
    if force {
        delete_ui_subtree(e, key, out)?;
        return Ok(());
    }
    let cb = match &node.specific {
        UiSpecific::Figure {
            close_request_fcn, ..
        } => close_request_fcn.clone(),
        _ => None,
    };
    if let Some(cb) = cb {
        let src = super::graphics_handles::dot_syntax::handle_value(h.clone(), &e.cancel)?;
        let event = callbacks::make_event(e, src.clone(), "CloseRequest", &[])?;
        callbacks::invoke_callback(e, "CloseRequestFcn", &cb, src, event, out)?;
    } else {
        delete_ui_subtree(e, key, out)?;
    }
    Ok(())
}

/// clf(uifigure): deletes every component of the uifigure, keeps the figure.
pub(in crate::engine) fn clear_figure_components(e: &mut Engine, h: &GraphicsHandle) -> Result<()> {
    let key = h.object.recovery_key();
    let Some(node) = e.ui.get_key(key) else {
        return Err("clf target must be a live figure handle".into());
    };
    if node.kind != UiKind::Figure {
        return Err("clf target must be a live figure handle".into());
    }
    let children = node.children.clone();
    for child in children {
        e.check_cancel()?;
        e.ui.delete_subtree(child);
    }
    Ok(())
}

pub(in crate::engine) fn close_all(e: &mut Engine, out: &mut Execution) {
    let figs: Vec<usize> = e.ui.figures.clone();
    for key in figs {
        let _ = delete_ui_subtree(e, key, out);
    }
    let classic_keys: Vec<usize> = e.ui.classic_fig_children.keys().copied().collect();
    for fk in classic_keys {
        e.ui.delete_classic_figure_children(fk);
    }
    e.ui.guidata.clear();
    e.ui.pending_alert_close.clear();
    e.ui.pending_alert_figs.clear();
    e.ui.active_confirm_figs.clear();
}

/// Dispatch one UI event, reporting any non-cancellation error as a MATLAB callback
/// warning so callers draining a queue can continue to subsequent events.
pub fn dispatch_ui_event_or_warn(
    e: &mut Engine,
    ev: snapshot::UiEvent,
    out: &mut Execution,
) -> Result<()> {
    if let Err(err) = snapshot::dispatch_ui_event(e, ev, out) {
        e.check_cancel()?;
        e.clear_raised_error();
        let msg = format!("Error executing UI callback: {err}");
        let _ = e.lng_warn(&msg, "MATLAB:ui:CallbackError", out);
    }
    Ok(())
}

/// Drain and dispatch all queued interactive UI events from the host (used by `drawnow`
/// and `pause`). Continues past callback errors and flushes a frame if any event ran.
pub(in crate::engine) fn drain_ui_events(e: &mut Engine, out: &mut Execution) -> Result<()> {
    let mut dispatched = false;
    loop {
        e.check_cancel()?;
        let next = e
            .host_io
            .clone()
            .and_then(|host| host.lock().ok().and_then(|mut h| h.try_poll_ui_event()));
        let Some(ev) = next else {
            break;
        };
        dispatched = true;
        dispatch_ui_event_or_warn(e, ev, out)?;
    }
    if dispatched {
        e.gobj_flush_frame();
    }
    Ok(())
}

/// Handle `uiwait` for UI figures (and classic figures with UI controls).
/// Returns `Ok(Some(()))` if handled, `Ok(None)` to fall back to dialogs.
pub(in crate::engine) fn uiwait(
    e: &mut Engine,
    args: &[Value],
    out: &mut Execution,
) -> Result<Option<()>> {
    if args.len() > 2 {
        return Err("Too many input arguments.".into());
    }
    let timeout_secs: Option<f64> = if let Some(timeout) = args.get(1) {
        match timeout {
            Value::Matrix(m) if m.len() == 1 && m.data()[0] >= 0. => Some(m.data()[0]),
            _ => return Err("The timeout must be a nonnegative scalar.".into()),
        }
    } else {
        None
    };

    let (key, is_classic): (usize, bool) = if args.is_empty() {
        if let Some(&k) = e.ui.figures.last() {
            (k, false)
        } else {
            return Ok(None);
        }
    } else {
        let Value::Container(Array::Graphics(arr)) = &args[0] else {
            return Ok(None);
        };
        let h = arr.scalar_handle()?;
        match h.kind() {
            GraphicsKind::UiComponent => {
                let k = h.object.recovery_key();
                let Some(node) = e.ui.get_key(k) else {
                    return Err("Invalid or deleted object.".into());
                };
                if node.kind != UiKind::Figure {
                    return Err("uiwait requires a figure handle.".into());
                }
                (k, false)
            }
            GraphicsKind::Figure => {
                // msgbox, waitbar and other dialog handles are figure-kind but
                // not live plot figures. Fall back so dlg_uiwait waits on the
                // dialog instead of reporting "Graphics figure is no longer live".
                let Some(k) = live_plot_figure_key(e, h)? else {
                    return Ok(None);
                };
                if e.ui.has_classic_ui(k) {
                    (k, true)
                } else {
                    return Ok(None);
                }
            }
            _ => return Ok(None),
        }
    };

    let interactive = e
        .host_io
        .as_ref()
        .and_then(|host| host.lock().ok().map(|h| h.supports_ui_wait()))
        .unwrap_or(false);
    if !interactive {
        // In -batch / headless, no interactive event loop is blocking; return immediately.
        e.ui.waiting.remove(&key);
        return Ok(Some(()));
    }

    e.ui.waiting.insert(key);
    e.gobj_flush_frame();
    let deadline = timeout_secs.and_then(|s| {
        std::time::Instant::now().checked_add(std::time::Duration::from_secs_f64(s.min(1e7)))
    });
    loop {
        e.check_cancel()?;
        if !e.ui.waiting.contains(&key) {
            break;
        }
        let still_live = if is_classic {
            e.figures().any(|(_, f)| f.identity.recovery_key() == key)
        } else {
            e.ui.get_key(key).is_some()
        };
        if !still_live {
            break;
        }
        if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            break;
        }
        let next_event = if let Some(host) = e.host_io.clone() {
            host.lock()
                .ok()
                .and_then(|mut h| h.poll_ui_event(std::time::Duration::from_millis(10)))
        } else {
            None
        };
        if let Some(ev) = next_event {
            dispatch_ui_event_or_warn(e, ev, out)?;
            e.gobj_flush_frame();
        }
    }
    e.ui.waiting.remove(&key);
    Ok(Some(()))
}

pub(in crate::engine) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    if !handles(name) {
        return Ok(None);
    }
    e.check_cancel()?;
    let val = match name {
        "uifigure" => create_uifigure(e, args)?,
        "uigridlayout" => create_uigridlayout(e, args)?,
        "uipanel" => create_simple(e, args, UiKind::Panel, None)?,
        "uitabgroup" => create_simple(e, args, UiKind::TabGroup, None)?,
        "uitab" => create_uitab(e, args)?,
        "uibutton" => create_uibutton(e, args)?,
        "uilabel" => create_simple(e, args, UiKind::Label, None)?,
        "uieditfield" => create_uieditfield(e, args)?,
        "uitextarea" => create_simple(e, args, UiKind::TextArea, None)?,
        "uidropdown" => create_simple(e, args, UiKind::DropDown, None)?,
        "uilistbox" => create_simple(e, args, UiKind::ListBox, None)?,
        "uicheckbox" => create_simple(e, args, UiKind::CheckBox, None)?,
        "uibuttongroup" => create_simple(e, args, UiKind::ButtonGroup, None)?,
        "uiradiobutton" => create_uiradiobutton(e, args)?,
        "uislider" => create_uislider(e, args)?,
        "uispinner" => create_simple(e, args, UiKind::Spinner, None)?,
        "uiswitch" => create_uiswitch(e, args)?,
        "uitable" => create_simple(e, args, UiKind::Table, None)?,
        "uiaxes" => create_uiaxes(e, args)?,
        "uialert" => dialogs_ui::call_uialert(e, args, out)?,
        "uiconfirm" => dialogs_ui::call_uiconfirm(e, args, out)?,
        "uiprogressdlg" => dialogs_ui::call_uiprogressdlg(e, args, out)?,
        "uicontrol" => classic::create_uicontrol(e, args)?,
        "gcbo" => classic::call_gcbo(e, args)?,
        "gcbf" => classic::call_gcbf(e, args)?,
        "guidata" => classic::call_guidata(e, args)?,
        "uiresume" => {
            call_uiresume(e, args)?;
            Value::Void
        }
        "mrUiFire" => {
            callbacks::mr_ui_fire(e, args, out)?;
            Value::Void
        }
        _ => return Ok(None),
    };
    Ok(Some(val))
}

/// Recovery key of a live plotting figure, or `Ok(None)` when `h` is a
/// figure-kind dialog handle (msgbox, waitbar) or is otherwise not a live plot.
/// Lookup failures that mean "not live" do not propagate, so `uiwait` can fall
/// back to the dialog path and `uiresume` can stay a quiet no-op.
fn live_plot_figure_key(e: &Engine, h: &GraphicsHandle) -> Result<Option<usize>> {
    let located = match e.locate_graphics(h) {
        Ok(located) => located,
        Err(err) if graphics_not_live(&err) => return Ok(None),
        Err(err) => return Err(err),
    };
    match e.graphics_figure(located.figure) {
        Ok(fig) => Ok(Some(fig.identity.recovery_key())),
        Err(err) if graphics_not_live(&err) => Ok(None),
        Err(err) => Err(err),
    }
}

fn graphics_not_live(err: &str) -> bool {
    err == "Graphics figure is no longer live"
        || err == "Graphics handle is stale or does not belong to this engine graph"
        || err == "Invalid or deleted object."
}

fn call_uiresume(e: &mut Engine, args: &[Value]) -> Result<()> {
    if args.len() > 1 {
        return Err("Too many input arguments.".into());
    }
    if let Some(arg) = args.first() {
        let Value::Container(Array::Graphics(arr)) = arg else {
            return Err("uiresume requires a figure handle.".into());
        };
        let h = arr.scalar_handle()?;
        match h.kind() {
            GraphicsKind::UiComponent => {
                let key = h.object.recovery_key();
                let Some(node) = e.ui.get_key(key) else {
                    return Err("Invalid or deleted object.".into());
                };
                if node.kind != UiKind::Figure {
                    return Err("uiresume requires a figure handle.".into());
                }
                e.ui.waiting.remove(&key);
            }
            GraphicsKind::Figure => {
                // Dialog handles stay a quiet no-op. A live classic figure
                // (including gcbf) is removed from `waiting`.
                if let Some(key) = live_plot_figure_key(e, h)? {
                    e.ui.waiting.remove(&key);
                }
            }
            _ => return Err("uiresume requires a figure handle.".into()),
        }
    } else if let Some(&fig) = e.ui.figures.last()
        && e.ui.waiting.contains(&fig)
    {
        e.ui.waiting.remove(&fig);
    } else if let Ok(fig) = e.graphics_figure(e.current_figure)
        && e.ui.waiting.contains(&fig.identity.recovery_key())
    {
        let key = fig.identity.recovery_key();
        e.ui.waiting.remove(&key);
    }
    Ok(())
}

fn as_text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.is_text_row() => {
            c.to_utf8(&std::sync::atomic::AtomicBool::new(false)).ok()
        }
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            s.data()[0].as_ref().map(|x| x.to_string())
        }
        _ => None,
    }
}

/// Finds a trailing `'Parent', value` pair (names are aligned from the end, so
/// leading positional arguments such as a grid size are skipped). Returns the
/// parent value and the arguments without that pair.
fn take_parent_pair(args: &[Value]) -> Option<(Value, Vec<Value>)> {
    let mut i = args.len().checked_sub(2)?;
    loop {
        match as_text(&args[i]) {
            Some(name) if name.eq_ignore_ascii_case("parent") => {
                let mut rest = args.to_vec();
                let value = rest.remove(i + 1);
                rest.remove(i);
                return Some((value, rest));
            }
            Some(_) => {}
            None => return None,
        }
        i = i.checked_sub(2)?;
    }
}

fn extract_parent_arg<'a>(
    e: &Engine,
    args: &'a [Value],
) -> Result<(Option<usize>, std::borrow::Cow<'a, [Value]>)> {
    if !matches!(args.first(), Some(Value::Container(Array::Graphics(_))))
        && let Some((parent, mut rest)) = take_parent_pair(args)
    {
        rest.insert(0, parent);
        let (key, _) = extract_parent_arg(e, &rest[..1])?;
        if key.is_none() {
            return Err("Parent must be a UI figure or container handle.".into());
        }
        rest.remove(0);
        return Ok((key, std::borrow::Cow::Owned(rest)));
    }
    if let Some(Value::Container(Array::Graphics(arr))) = args.first() {
        let h = arr.scalar_handle()?;
        if h.kind() != GraphicsKind::UiComponent {
            return Err("Parent must be a UI figure or container handle.".into());
        }
        let key = h.object.recovery_key();
        let Some(node) = e.ui.get_key(key) else {
            return Err("Invalid or deleted parent object.".into());
        };
        if !node.kind.is_container() {
            return Err(format!(
                "Cannot add children to component of type '{}'.",
                node.kind.type_name()
            ));
        }
        Ok((Some(key), std::borrow::Cow::Borrowed(&args[1..])))
    } else {
        Ok((None, std::borrow::Cow::Borrowed(args)))
    }
}

fn ensure_default_figure(e: &mut Engine) -> Result<usize> {
    let node = UiNode::new(
        UiKind::Figure,
        None,
        UiSpecific::Figure {
            name: String::new(),
            color: [0.94, 0.94, 0.94],
            close_request_fcn: None,
            is_uifigure: true,
        },
    );
    e.ui.insert_node(node)
}

fn default_specific(kind: UiKind) -> UiSpecific {
    match kind {
        UiKind::Figure => UiSpecific::Figure {
            name: String::new(),
            color: [0.94, 0.94, 0.94],
            close_request_fcn: None,
            is_uifigure: true,
        },
        UiKind::GridLayout => UiSpecific::GridLayout {
            row_height: vec![GridTrack::Weight(1.0); 2],
            column_width: vec![GridTrack::Weight(1.0); 2],
            padding: [10., 10., 10., 10.],
            row_spacing: 10.0,
            column_spacing: 10.0,
            scrollable: false,
        },
        UiKind::Panel => UiSpecific::Panel {
            title: String::new(),
            title_position: "lefttop".into(),
            border_type: "line".into(),
        },
        UiKind::TabGroup => UiSpecific::TabGroup {
            selected_tab: None,
            selection_changed_fcn: None,
            tab_location: "top".into(),
        },
        UiKind::Tab => UiSpecific::Tab {
            title: String::new(),
        },
        UiKind::Button => UiSpecific::Button {
            text: "Button".into(),
            button_pushed_fcn: None,
        },
        UiKind::StateButton => UiSpecific::StateButton {
            text: "Button".into(),
            value: false,
            value_changed_fcn: None,
        },
        UiKind::Label => UiSpecific::Label {
            text: "Label".into(),
            horizontal_alignment: "left".into(),
            vertical_alignment: "center".into(),
            word_wrap: false,
        },
        UiKind::EditField => UiSpecific::EditField {
            value: String::new(),
            editable: true,
            placeholder: String::new(),
            value_changed_fcn: None,
            value_changing_fcn: None,
        },
        UiKind::NumericEditField => UiSpecific::NumericEditField {
            value: 0.0,
            limits: [f64::NEG_INFINITY, f64::INFINITY],
            value_display_format: "%11.4g".into(),
            editable: true,
            lower_limit_inclusive: true,
            upper_limit_inclusive: true,
            round_fractional_values: false,
            value_changed_fcn: None,
            value_changing_fcn: None,
        },
        UiKind::TextArea => UiSpecific::TextArea {
            value: vec![String::new()],
            editable: true,
            word_wrap: true,
            value_changed_fcn: None,
            value_changing_fcn: None,
        },
        UiKind::DropDown => UiSpecific::DropDown {
            items: vec!["Option 1".into(), "Option 2".into(), "Option 3".into()],
            items_data: None,
            value: Value::Text("Option 1".into()),
            editable: false,
            value_changed_fcn: None,
        },
        UiKind::ListBox => UiSpecific::ListBox {
            items: vec![
                "Item 1".into(),
                "Item 2".into(),
                "Item 3".into(),
                "Item 4".into(),
            ],
            items_data: None,
            value: Value::Text("Item 1".into()),
            multiselect: false,
            value_changed_fcn: None,
        },
        UiKind::CheckBox => UiSpecific::CheckBox {
            text: "Check Box".into(),
            value: false,
            value_changed_fcn: None,
        },
        UiKind::ButtonGroup => UiSpecific::ButtonGroup {
            title: String::new(),
            selected_object: None,
            selection_changed_fcn: None,
        },
        UiKind::RadioButton => UiSpecific::RadioButton {
            text: "Radio Button".into(),
            value: false,
        },
        UiKind::Slider => UiSpecific::Slider {
            limits: [0.0, 100.0],
            value: 0.0,
            major_ticks: vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0],
            major_ticks_manual: false,
            orientation: "horizontal".into(),
            value_changed_fcn: None,
            value_changing_fcn: None,
        },
        UiKind::Spinner => UiSpecific::Spinner {
            limits: [f64::NEG_INFINITY, f64::INFINITY],
            step: 1.0,
            value: 0.0,
            value_display_format: "%11.4g".into(),
            editable: true,
            value_changed_fcn: None,
            value_changing_fcn: None,
        },
        UiKind::Switch => UiSpecific::Switch {
            style: "slider".into(),
            items: ["Off".into(), "On".into()],
            items_data: None,
            value: Value::Text("Off".into()),
            orientation: "horizontal".into(),
            value_changed_fcn: None,
        },
        UiKind::Table => UiSpecific::Table {
            data: Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new()).unwrap()),
            column_name: TableHeader::Numbered,
            row_name: TableHeader::Numbered,
            column_editable: vec![false],
            column_width: TableColumnWidth::Auto,
            cell_edit_callback: None,
            cell_selection_callback: None,
        },
        UiKind::Axes => unreachable!("uiaxes uses create_uiaxes"),
        UiKind::ProgressDialog => unreachable!("uiprogressdlg uses dialogs_ui::call_uiprogressdlg"),
    }
}

fn create_uiaxes(e: &mut Engine, args: &[Value]) -> Result<Value> {
    let mut ui_parent: Option<usize> = None;
    let mut classic_parent: Option<ObjectIdentity> = None;
    // A 'Parent' pair is handled like the positional parent.
    let moved;
    let args = match take_parent_pair(args) {
        Some((parent, mut rest))
            if !matches!(args.first(), Some(Value::Container(Array::Graphics(_)))) =>
        {
            rest.insert(0, parent);
            moved = rest;
            &moved[..]
        }
        _ => args,
    };
    let mut rest = args;

    if let Some(Value::Container(Array::Graphics(arr))) = args.first() {
        let h = arr.scalar_handle()?;
        match h.kind() {
            GraphicsKind::UiComponent => {
                let pk = h.object.recovery_key();
                let Some(node) = e.ui.get_key(pk) else {
                    return Err("Invalid or deleted parent object.".into());
                };
                if !node.kind.is_container() {
                    return Err(format!(
                        "Cannot add uiaxes to component of type '{}'.",
                        node.kind.type_name()
                    ));
                }
                ui_parent = Some(pk);
            }
            GraphicsKind::Figure => {
                let located = e.locate_graphics(h)?;
                let fig = e.graphics_figure(located.figure)?;
                classic_parent = Some(fig.identity.clone());
            }
            _ => return Err("Parent of uiaxes must be a figure or UI container handle.".into()),
        }
        rest = &args[1..];
    }

    if !rest.len().is_multiple_of(2) {
        return Err("uiaxes expects Name, Value property pairs.".into());
    }

    if ui_parent.is_none() && classic_parent.is_none() {
        ui_parent = Some(ensure_default_figure(e)?);
    }

    let fig_id = UIAXES_FIRST_FIG_ID
        .checked_add(e.ui.next_uiaxes_fig_id)
        .ok_or("Too many uiaxes instances.")?;
    e.ui.next_uiaxes_fig_id += 1;

    let mut backing_fig = crate::plot::Figure::default();
    let _ = e.apply_figure_defaults(&mut backing_fig);
    let _ = e.apply_axes_defaults(&mut backing_fig);
    let fig_identity = backing_fig.identity.clone();
    let axes_identity = backing_fig.axes.identity.clone();

    e.ui.uiaxes_figures.insert(fig_id, backing_fig);

    let mut node = UiNode::new(
        UiKind::Axes,
        ui_parent,
        UiSpecific::Axes {
            fig_id,
            fig_identity: fig_identity.clone(),
        },
    );
    node.identity = axes_identity.clone();
    node.classic_parent = classic_parent.clone();

    let key = match e.ui.insert_node(node) {
        Ok(k) => k,
        Err(err) => {
            e.ui.uiaxes_figures.remove(&fig_id);
            return Err(err);
        }
    };
    if let Some(ref cid) = classic_parent {
        e.ui.classic_fig_children
            .entry(cid.recovery_key())
            .or_default()
            .push(key);
    }

    let handle = GraphicsHandle::axes(&fig_identity, &axes_identity);
    if !rest.is_empty() {
        for pair in rest.as_chunks::<2>().0 {
            let name = match as_text(&pair[0]) {
                Some(s) => s,
                None => {
                    e.ui.delete_subtree(key);
                    return Err("Property name must be character text.".into());
                }
            };
            let res = if is_uiaxes_ui_property(&name) {
                props::set_properties(e, &handle, pair)
            } else {
                let located = e.locate_graphics(&handle)?;
                let mut norm = e.text_arguments(pair.to_vec())?;
                super::graphics_handles::preserve_string_property_values(pair, &mut norm);
                e.set_graphics_properties(&handle, located, &norm)
            };
            if let Err(err) = res {
                e.ui.delete_subtree(key);
                return Err(err);
            }
        }
    }

    super::graphics_handles::dot_syntax::handle_value(handle, &e.cancel)
}

fn finish_create(e: &mut Engine, key: usize, nv_pairs: &[Value]) -> Result<Value> {
    let handle =
        e.ui.get_key(key)
            .ok_or("Failed to create UI component")?
            .handle();
    if !nv_pairs.is_empty()
        && let Err(err) = props::apply_creation_pairs(e, &handle, nv_pairs)
    {
        e.ui.delete_subtree(key);
        return Err(err);
    }
    super::graphics_handles::dot_syntax::handle_value(handle, &e.cancel)
}

fn create_uifigure(e: &mut Engine, args: &[Value]) -> Result<Value> {
    if !args.len().is_multiple_of(2) {
        return Err("uifigure expects Name, Value property pairs.".into());
    }
    let key = ensure_default_figure(e)?;
    finish_create(e, key, args)
}

fn create_uigridlayout(e: &mut Engine, args: &[Value]) -> Result<Value> {
    let (parent_opt, rest_buf) = extract_parent_arg(e, args)?;
    let mut rest: &[Value] = &rest_buf;
    let (mut rows, mut cols) = (2usize, 2usize);
    if let Some(Value::Matrix(m)) = rest.first() {
        if m.len() != 2 || (m.rows() != 1 && m.cols() != 1) {
            return Err(
                "Grid size must be a two-element positive integer vector [rows cols].".into(),
            );
        }
        let r = m.data()[0];
        let c = m.data()[1];
        if !r.is_finite()
            || !c.is_finite()
            || r < 1.0
            || c < 1.0
            || r.fract() != 0.0
            || c.fract() != 0.0
            || r as usize > MAX_GRID_TRACKS
            || c as usize > MAX_GRID_TRACKS
        {
            return Err(format!(
                "Grid size must contain integers in 1..={MAX_GRID_TRACKS}."
            ));
        }
        rows = r as usize;
        cols = c as usize;
        rest = &rest[1..];
    }
    if !rest.len().is_multiple_of(2) {
        return Err("uigridlayout expects Name, Value property pairs.".into());
    }
    let parent = match parent_opt {
        Some(pk) => pk,
        None => ensure_default_figure(e)?,
    };
    let specific = UiSpecific::GridLayout {
        row_height: vec![GridTrack::Weight(1.0); rows],
        column_width: vec![GridTrack::Weight(1.0); cols],
        padding: [10., 10., 10., 10.],
        row_spacing: 10.0,
        column_spacing: 10.0,
        scrollable: false,
    };
    let node = UiNode::new(UiKind::GridLayout, Some(parent), specific);
    let key = e.ui.insert_node(node)?;
    finish_create(e, key, rest)
}

fn create_simple(
    e: &mut Engine,
    args: &[Value],
    kind: UiKind,
    override_specific: Option<UiSpecific>,
) -> Result<Value> {
    let (parent_opt, rest_buf) = extract_parent_arg(e, args)?;
    let rest: &[Value] = &rest_buf;
    if !rest.len().is_multiple_of(2) {
        return Err(format!(
            "{} expects Name, Value property pairs.",
            kind.type_name()
        ));
    }
    let parent = match parent_opt {
        Some(pk) => pk,
        None => ensure_default_figure(e)?,
    };
    let specific = override_specific.unwrap_or_else(|| default_specific(kind));
    let node = UiNode::new(kind, Some(parent), specific);
    let key = e.ui.insert_node(node)?;
    finish_create(e, key, rest)
}

fn create_uitab(e: &mut Engine, args: &[Value]) -> Result<Value> {
    let (parent_opt, rest_buf) = extract_parent_arg(e, args)?;
    let rest: &[Value] = &rest_buf;
    if !rest.len().is_multiple_of(2) {
        return Err("uitab expects Name, Value property pairs.".into());
    }
    let parent = match parent_opt {
        Some(pk) => {
            if e.ui.get_key(pk).is_none_or(|n| n.kind != UiKind::TabGroup) {
                return Err("Parent of uitab must be a TabGroup.".into());
            }
            pk
        }
        None => {
            let fig = ensure_default_figure(e)?;
            let tg = UiNode::new(
                UiKind::TabGroup,
                Some(fig),
                default_specific(UiKind::TabGroup),
            );
            e.ui.insert_node(tg)?
        }
    };
    let node = UiNode::new(UiKind::Tab, Some(parent), default_specific(UiKind::Tab));
    let key = e.ui.insert_node(node)?;
    finish_create(e, key, rest)
}

fn create_uiradiobutton(e: &mut Engine, args: &[Value]) -> Result<Value> {
    let (parent_opt, rest_buf) = extract_parent_arg(e, args)?;
    let rest: &[Value] = &rest_buf;
    if !rest.len().is_multiple_of(2) {
        return Err("uiradiobutton expects Name, Value property pairs.".into());
    }
    let parent = match parent_opt {
        Some(pk) => {
            if e.ui
                .get_key(pk)
                .is_none_or(|n| n.kind != UiKind::ButtonGroup)
            {
                return Err("Parent of uiradiobutton must be a ButtonGroup.".into());
            }
            pk
        }
        None => {
            let fig = ensure_default_figure(e)?;
            let bg = UiNode::new(
                UiKind::ButtonGroup,
                Some(fig),
                default_specific(UiKind::ButtonGroup),
            );
            e.ui.insert_node(bg)?
        }
    };
    let node = UiNode::new(
        UiKind::RadioButton,
        Some(parent),
        default_specific(UiKind::RadioButton),
    );
    let key = e.ui.insert_node(node)?;
    finish_create(e, key, rest)
}

fn create_uibutton(e: &mut Engine, args: &[Value]) -> Result<Value> {
    let (parent_opt, rest_buf) = extract_parent_arg(e, args)?;
    let mut rest: &[Value] = &rest_buf;
    let mut kind = UiKind::Button;
    if !rest.len().is_multiple_of(2)
        && let Some(style) = rest.first().and_then(as_text)
    {
        match style.to_ascii_lowercase().as_str() {
            "push" => kind = UiKind::Button,
            "state" => kind = UiKind::StateButton,
            _ => {
                return Err(format!(
                    "Invalid uibutton style '{style}'; use 'push' or 'state'."
                ));
            }
        }
        rest = &rest[1..];
    }
    if !rest.len().is_multiple_of(2) {
        return Err("uibutton expects Name, Value property pairs.".into());
    }
    let parent = match parent_opt {
        Some(pk) => pk,
        None => ensure_default_figure(e)?,
    };
    let node = UiNode::new(kind, Some(parent), default_specific(kind));
    let key = e.ui.insert_node(node)?;
    finish_create(e, key, rest)
}

fn create_uieditfield(e: &mut Engine, args: &[Value]) -> Result<Value> {
    let (parent_opt, rest_buf) = extract_parent_arg(e, args)?;
    let mut rest: &[Value] = &rest_buf;
    let mut kind = UiKind::EditField;
    if !rest.len().is_multiple_of(2)
        && let Some(style) = rest.first().and_then(as_text)
    {
        match style.to_ascii_lowercase().as_str() {
            "text" => kind = UiKind::EditField,
            "numeric" => kind = UiKind::NumericEditField,
            _ => {
                return Err(format!(
                    "Invalid uieditfield style '{style}'; use 'text' or 'numeric'."
                ));
            }
        }
        rest = &rest[1..];
    }
    if !rest.len().is_multiple_of(2) {
        return Err("uieditfield expects Name, Value property pairs.".into());
    }
    let parent = match parent_opt {
        Some(pk) => pk,
        None => ensure_default_figure(e)?,
    };
    let node = UiNode::new(kind, Some(parent), default_specific(kind));
    let key = e.ui.insert_node(node)?;
    finish_create(e, key, rest)
}

fn create_uislider(e: &mut Engine, args: &[Value]) -> Result<Value> {
    let (parent_opt, rest_buf) = extract_parent_arg(e, args)?;
    let mut rest: &[Value] = &rest_buf;
    if !rest.len().is_multiple_of(2)
        && let Some(style) = rest.first().and_then(as_text)
    {
        if !style.eq_ignore_ascii_case("slider") {
            return Err(format!("Invalid uislider style '{style}'; use 'slider'."));
        }
        rest = &rest[1..];
    }
    if !rest.len().is_multiple_of(2) {
        return Err("uislider expects Name, Value property pairs.".into());
    }
    let parent = match parent_opt {
        Some(pk) => pk,
        None => ensure_default_figure(e)?,
    };
    let node = UiNode::new(
        UiKind::Slider,
        Some(parent),
        default_specific(UiKind::Slider),
    );
    let key = e.ui.insert_node(node)?;
    finish_create(e, key, rest)
}

fn create_uiswitch(e: &mut Engine, args: &[Value]) -> Result<Value> {
    let (parent_opt, rest_buf) = extract_parent_arg(e, args)?;
    let mut rest: &[Value] = &rest_buf;
    let mut style_str = "slider".to_string();
    if !rest.len().is_multiple_of(2)
        && let Some(style) = rest.first().and_then(as_text)
    {
        let lower = style.to_ascii_lowercase();
        if !matches!(lower.as_str(), "slider" | "rocker" | "toggle") {
            return Err(format!(
                "Invalid uiswitch style '{style}'; use 'slider', 'rocker', or 'toggle'."
            ));
        }
        style_str = lower;
        rest = &rest[1..];
    }
    if !rest.len().is_multiple_of(2) {
        return Err("uiswitch expects Name, Value property pairs.".into());
    }
    let parent = match parent_opt {
        Some(pk) => pk,
        None => ensure_default_figure(e)?,
    };
    let mut specific = default_specific(UiKind::Switch);
    if let UiSpecific::Switch { style, .. } = &mut specific {
        *style = style_str;
    }
    let node = UiNode::new(UiKind::Switch, Some(parent), specific);
    let key = e.ui.insert_node(node)?;
    finish_create(e, key, rest)
}

/// Utility: build a 1x1 `StructArray` from `(field_name, Value)` pairs.
pub(super) fn make_struct(e: &Engine, fields: Vec<(&str, Value)>) -> Result<Value> {
    let names: Vec<String> = fields.iter().map(|(k, _)| (*k).to_string()).collect();
    let mut slots = Vec::with_capacity(fields.len());
    for (_, v) in fields {
        slots.push(Arc::new(v.array(&e.cancel)?));
    }
    Ok(Value::Container(Array::Struct(
        crate::StructArray::from_slots(1, 1, names, slots, &e.cancel)?,
    )))
}
