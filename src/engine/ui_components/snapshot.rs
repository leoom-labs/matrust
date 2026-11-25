//! Immutable UI snapshot for desktop rendering and interactive UI event dispatch.
//!
//! Snapshots are built on the worker thread and sent to the desktop UI; user
//! interactions come back as [`UiEvent`] values and run on the engine thread
//! through the exact same callback path as `mrUiFire`.
use super::callbacks::{
    fire_cancel, fire_choose, fire_edit, fire_press, fire_select, fire_slide, fire_toggle,
    fire_type, invoke_callback, make_event,
};
use super::props::values_equal;
use super::{
    GridTrack, TableColumnWidth, TableHeader, UiKind, UiLayout, UiNode, UiSpecific, as_text,
    close_figure,
};
use crate::engine::{Engine, Execution, Value};
use crate::{Array, CellArray, CharacterRow, LogicalMatrix, Matrix, Result};
use std::collections::BTreeMap;
use std::sync::Arc;

/// A single cell value in a rendered `uitable`.
#[derive(Clone, Debug, PartialEq)]
pub enum UiTableCell {
    Numeric(f64),
    Logical(bool),
    Text(String),
}

/// Widget-specific display state captured for one [`UiNode`].
#[derive(Clone, Debug)]
pub enum UiWidgetSnapshot {
    Figure,
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
        tab_location: String,
    },
    Tab {
        title: String,
    },
    Button {
        text: String,
    },
    StateButton {
        text: String,
        value: bool,
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
    },
    NumericEditField {
        value: f64,
        limits: [f64; 2],
        value_display_format: String,
        editable: bool,
        lower_limit_inclusive: bool,
        upper_limit_inclusive: bool,
        round_fractional_values: bool,
    },
    TextArea {
        value: Vec<String>,
        editable: bool,
        word_wrap: bool,
    },
    DropDown {
        items: Vec<String>,
        selected_index: Option<usize>,
        display_text: String,
        editable: bool,
    },
    ListBox {
        items: Vec<String>,
        selected_indices: Vec<usize>,
        multiselect: bool,
    },
    CheckBox {
        text: String,
        value: bool,
    },
    ButtonGroup {
        title: String,
        selected_object: Option<usize>,
    },
    RadioButton {
        text: String,
        value: bool,
    },
    Slider {
        limits: [f64; 2],
        value: f64,
        major_ticks: Vec<f64>,
        orientation: String,
    },
    Spinner {
        limits: [f64; 2],
        step: f64,
        value: f64,
        value_display_format: String,
        editable: bool,
    },
    Switch {
        style: String,
        items: [String; 2],
        selected_index: usize,
        orientation: String,
    },
    Table {
        rows: usize,
        cols: usize,
        /// Shared, so cloning a node per frame does not copy the cells.
        cells: std::sync::Arc<Vec<Vec<UiTableCell>>>,
        col_names: Vec<String>,
        row_names: Vec<String>,
        col_editable: Vec<bool>,
        col_widths: Vec<GridTrack>,
    },
    Axes {
        fig_id: u32,
        figure: Option<Box<crate::plot::Figure>>,
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
    },
}

/// Snapshot of a single UI component node.
#[derive(Clone, Debug)]
pub struct UiNodeSnapshot {
    pub key: usize,
    pub kind: UiKind,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub tag: String,
    pub enable: bool,
    pub visible: bool,
    pub position: [f64; 4],
    pub units: String,
    pub layout: Option<UiLayout>,
    pub font_size: f64,
    pub background_color: [f64; 3],
    pub tooltip: String,
    pub is_classic: bool,
    pub widget: UiWidgetSnapshot,
}

/// Snapshot of one top-level UI figure window (modern `uifigure` or classic `figure` with controls).
#[derive(Clone, Debug)]
pub struct UiWindowSnapshot {
    pub fig_key: usize,
    pub is_classic: bool,
    pub classic_fig_id: Option<u32>,
    pub title: String,
    pub position: [f64; 4],
    pub color: [f64; 3],
    pub root_children: Vec<usize>,
    pub progress_dialogs: Vec<usize>,
}

/// Complete snapshot of all visible UI windows and their component trees.
#[derive(Clone, Debug, Default)]
pub struct UiSnapshot {
    pub windows: Vec<UiWindowSnapshot>,
    pub nodes: BTreeMap<usize, UiNodeSnapshot>,
}

/// User interaction sent from the desktop UI thread to the engine worker thread.
#[derive(Clone, Debug)]
pub enum UiEvent {
    Press {
        key: usize,
    },
    TypeText {
        key: usize,
        text: String,
    },
    TypeNumeric {
        key: usize,
        value: f64,
    },
    TypeTextArea {
        key: usize,
        lines: Vec<String>,
    },
    ChooseIndex {
        key: usize,
        index: usize,
    },
    ChooseText {
        key: usize,
        text: String,
    },
    ChooseIndices {
        key: usize,
        indices: Vec<usize>,
    },
    ChooseChild {
        key: usize,
    },
    Slide {
        key: usize,
        value: f64,
    },
    Toggle {
        key: usize,
        value: bool,
    },
    EditTableCell {
        key: usize,
        row: usize,
        col: usize,
        value: UiTableCell,
    },
    SelectTableCell {
        key: usize,
        row: usize,
        col: usize,
    },
    CancelProgress {
        key: usize,
    },
    CloseFigure {
        fig_key: usize,
        is_classic: bool,
        classic_fig_id: Option<u32>,
    },
    AlertClosed {
        id: u32,
    },
}

fn find_selected_index(
    items: &[String],
    items_data: &Option<Vec<Value>>,
    value: &Value,
) -> Option<usize> {
    if let Some(data) = items_data {
        data.iter().position(|d| values_equal(d, value))
    } else if let Some(text) = as_text(value) {
        items.iter().position(|it| it == &text)
    } else {
        None
    }
}

fn find_listbox_indices(
    e: &Engine,
    items: &[String],
    items_data: &Option<Vec<Value>>,
    value: &Value,
    classic: Option<&super::ClassicMeta>,
) -> Vec<usize> {
    if let Some(meta) = classic {
        let idx = meta.value_num.round() as isize;
        if idx >= 1 && (idx as usize) <= items.len() {
            return vec![(idx - 1) as usize];
        }
        return Vec::new();
    }
    if let Value::Container(Array::Cell(cell)) = value {
        let mut out = Vec::new();
        for i in 0..cell.len() {
            if let Ok(elem) = cell.get_linear(i) {
                let v = Value::from(elem.as_ref().clone());
                if let Some(pos) = find_selected_index(items, items_data, &v)
                    && !out.contains(&pos)
                {
                    out.push(pos);
                }
            }
        }
        return out;
    }
    if let Value::Container(Array::Str(sa)) = value {
        let mut out = Vec::new();
        for s in sa.data() {
            if let Some(s) = s
                && let Some(pos) = items.iter().position(|it| it == s.as_ref())
                && !out.contains(&pos)
            {
                out.push(pos);
            }
        }
        return out;
    }
    let _ = e;
    find_selected_index(items, items_data, value)
        .into_iter()
        .collect()
}

fn array_to_cell(arr: &Array, cancel: &std::sync::atomic::AtomicBool) -> UiTableCell {
    match arr {
        Array::Numeric(m) if m.len() == 1 => UiTableCell::Numeric(m.data()[0]),
        Array::Logical(m) if m.len() == 1 => UiTableCell::Logical(m.data()[0] != 0),
        Array::Character(c) if c.is_text_row() => {
            UiTableCell::Text(c.to_utf8(cancel).unwrap_or_default())
        }
        Array::Str(s) if s.len() == 1 => {
            UiTableCell::Text(s.data()[0].as_deref().unwrap_or("").to_string())
        }
        other => UiTableCell::Text(other.preview(64)),
    }
}

fn snapshot_table_data(
    e: &Engine,
    data: &Value,
    column_name: &TableHeader,
    row_name: &TableHeader,
    column_editable: &[bool],
    column_width: &TableColumnWidth,
) -> UiWidgetSnapshot {
    let (rows, cols, cells, default_col_names) = match data {
        Value::Matrix(m) => {
            let (r, c) = (m.rows(), m.cols());
            let mut grid = Vec::with_capacity(r);
            for ri in 0..r {
                let mut row = Vec::with_capacity(c);
                for ci in 0..c {
                    row.push(UiTableCell::Numeric(m.get(ri, ci)));
                }
                grid.push(row);
            }
            (r, c, grid, None)
        }
        Value::Logical(m) => {
            let (r, c) = (m.rows(), m.cols());
            let mut grid = Vec::with_capacity(r);
            for ri in 0..r {
                let mut row = Vec::with_capacity(c);
                for ci in 0..c {
                    row.push(UiTableCell::Logical(m.data()[ci * r + ri] != 0));
                }
                grid.push(row);
            }
            (r, c, grid, None)
        }
        Value::Container(Array::Cell(cell)) => {
            let (r, c) = (cell.rows(), cell.cols());
            let mut grid = Vec::with_capacity(r);
            for ri in 0..r {
                let mut row = Vec::with_capacity(c);
                for ci in 0..c {
                    let item = cell
                        .get_linear(ci * r + ri)
                        .map(|a| array_to_cell(a.as_ref(), &e.cancel))
                        .unwrap_or_else(|_| UiTableCell::Text(String::new()));
                    row.push(item);
                }
                grid.push(row);
            }
            (r, c, grid, None)
        }
        Value::Container(Array::Table(tbl)) => {
            let parts = tbl.parts();
            let r = parts.height;
            let c = parts.columns.len();
            let mut grid = Vec::with_capacity(r);
            for ri in 0..r {
                let mut row = Vec::with_capacity(c);
                for col_arr in &parts.columns {
                    let cell_val = match col_arr {
                        Array::Numeric(m) if ri < m.rows() => UiTableCell::Numeric(m.get(ri, 0)),
                        Array::Logical(m) if ri < m.rows() => {
                            UiTableCell::Logical(m.data()[ri] != 0)
                        }
                        Array::Str(s) if ri < s.rows() => {
                            UiTableCell::Text(s.data()[ri].as_deref().unwrap_or("").to_string())
                        }
                        Array::Cell(cell) if ri < cell.len() => cell
                            .get_linear(ri)
                            .map(|a| array_to_cell(a.as_ref(), &e.cancel))
                            .unwrap_or_else(|_| UiTableCell::Text(String::new())),
                        _ => UiTableCell::Text(String::new()),
                    };
                    row.push(cell_val);
                }
                grid.push(row);
            }
            (r, c, grid, Some(parts.names.clone()))
        }
        _ => (0, 0, Vec::new(), None),
    };

    let col_names = match column_name {
        TableHeader::None => Vec::new(),
        TableHeader::Names(names) => names.clone(),
        TableHeader::Numbered => {
            if let Some(vn) = default_col_names {
                vn
            } else {
                (1..=cols).map(|i| i.to_string()).collect()
            }
        }
    };
    let row_names = match row_name {
        TableHeader::None => Vec::new(),
        TableHeader::Names(names) => names.clone(),
        TableHeader::Numbered => (1..=rows).map(|i| i.to_string()).collect(),
    };
    let col_editable = if column_editable.len() == 1 && cols > 1 {
        vec![column_editable[0]; cols]
    } else if column_editable.len() < cols {
        let mut v = column_editable.to_vec();
        v.resize(cols, false);
        v
    } else {
        column_editable.to_vec()
    };
    let col_widths = match column_width {
        TableColumnWidth::Auto => Vec::new(),
        TableColumnWidth::Tracks(t) => t.clone(),
    };

    UiWidgetSnapshot::Table {
        rows,
        cols,
        cells: std::sync::Arc::new(cells),
        col_names,
        row_names,
        col_editable,
        col_widths,
    }
}

fn snapshot_node(e: &Engine, key: usize, node: &UiNode) -> UiNodeSnapshot {
    let widget = match &node.specific {
        UiSpecific::Figure { .. } => UiWidgetSnapshot::Figure,
        UiSpecific::GridLayout {
            row_height,
            column_width,
            padding,
            row_spacing,
            column_spacing,
            scrollable,
        } => UiWidgetSnapshot::GridLayout {
            row_height: row_height.clone(),
            column_width: column_width.clone(),
            padding: *padding,
            row_spacing: *row_spacing,
            column_spacing: *column_spacing,
            scrollable: *scrollable,
        },
        UiSpecific::Panel {
            title,
            title_position,
            border_type,
        } => UiWidgetSnapshot::Panel {
            title: title.clone(),
            title_position: title_position.clone(),
            border_type: border_type.clone(),
        },
        UiSpecific::TabGroup {
            selected_tab,
            tab_location,
            ..
        } => UiWidgetSnapshot::TabGroup {
            selected_tab: *selected_tab,
            tab_location: tab_location.clone(),
        },
        UiSpecific::Tab { title } => UiWidgetSnapshot::Tab {
            title: title.clone(),
        },
        UiSpecific::Button { text, .. } => UiWidgetSnapshot::Button { text: text.clone() },
        UiSpecific::StateButton { text, value, .. } => UiWidgetSnapshot::StateButton {
            text: text.clone(),
            value: *value,
        },
        UiSpecific::Label {
            text,
            horizontal_alignment,
            vertical_alignment,
            word_wrap,
        } => UiWidgetSnapshot::Label {
            text: text.clone(),
            horizontal_alignment: horizontal_alignment.clone(),
            vertical_alignment: vertical_alignment.clone(),
            word_wrap: *word_wrap,
        },
        UiSpecific::EditField {
            value,
            editable,
            placeholder,
            ..
        } => UiWidgetSnapshot::EditField {
            value: value.clone(),
            editable: *editable,
            placeholder: placeholder.clone(),
        },
        UiSpecific::NumericEditField {
            value,
            limits,
            value_display_format,
            editable,
            lower_limit_inclusive,
            upper_limit_inclusive,
            round_fractional_values,
            ..
        } => UiWidgetSnapshot::NumericEditField {
            value: *value,
            limits: *limits,
            value_display_format: value_display_format.clone(),
            editable: *editable,
            lower_limit_inclusive: *lower_limit_inclusive,
            upper_limit_inclusive: *upper_limit_inclusive,
            round_fractional_values: *round_fractional_values,
        },
        UiSpecific::TextArea {
            value,
            editable,
            word_wrap,
            ..
        } => UiWidgetSnapshot::TextArea {
            value: value.clone(),
            editable: *editable,
            word_wrap: *word_wrap,
        },
        UiSpecific::DropDown {
            items,
            items_data,
            value,
            editable,
            ..
        } => {
            let selected_index = if let Some(meta) = &node.classic {
                let idx = meta.value_num.round() as isize;
                (idx >= 1 && (idx as usize) <= items.len()).then_some((idx - 1) as usize)
            } else {
                find_selected_index(items, items_data, value)
            };
            let display_text = selected_index
                .and_then(|i| items.get(i).cloned())
                .or_else(|| as_text(value))
                .unwrap_or_default();
            UiWidgetSnapshot::DropDown {
                items: items.clone(),
                selected_index,
                display_text,
                editable: *editable,
            }
        }
        UiSpecific::ListBox {
            items,
            items_data,
            value,
            multiselect,
            ..
        } => {
            let selected_indices =
                find_listbox_indices(e, items, items_data, value, node.classic.as_ref());
            UiWidgetSnapshot::ListBox {
                items: items.clone(),
                selected_indices,
                multiselect: *multiselect,
            }
        }
        UiSpecific::CheckBox { text, value, .. } => UiWidgetSnapshot::CheckBox {
            text: text.clone(),
            value: *value,
        },
        UiSpecific::ButtonGroup {
            title,
            selected_object,
            ..
        } => UiWidgetSnapshot::ButtonGroup {
            title: title.clone(),
            selected_object: *selected_object,
        },
        UiSpecific::RadioButton { text, value } => UiWidgetSnapshot::RadioButton {
            text: text.clone(),
            value: *value,
        },
        UiSpecific::Slider {
            limits,
            value,
            major_ticks,
            orientation,
            ..
        } => {
            let lim = if let Some(meta) = &node.classic {
                [meta.min, meta.max]
            } else {
                *limits
            };
            UiWidgetSnapshot::Slider {
                limits: lim,
                value: *value,
                major_ticks: major_ticks.clone(),
                orientation: orientation.clone(),
            }
        }
        UiSpecific::Spinner {
            limits,
            step,
            value,
            value_display_format,
            editable,
            ..
        } => UiWidgetSnapshot::Spinner {
            limits: *limits,
            step: *step,
            value: *value,
            value_display_format: value_display_format.clone(),
            editable: *editable,
        },
        UiSpecific::Switch {
            style,
            items,
            items_data,
            value,
            orientation,
            ..
        } => {
            let selected_index = find_selected_index(items, items_data, value).unwrap_or(0);
            UiWidgetSnapshot::Switch {
                style: style.clone(),
                items: items.clone(),
                selected_index,
                orientation: orientation.clone(),
            }
        }
        UiSpecific::Table {
            data,
            column_name,
            row_name,
            column_editable,
            column_width,
            ..
        } => snapshot_table_data(
            e,
            data,
            column_name,
            row_name,
            column_editable,
            column_width,
        ),
        UiSpecific::Axes { fig_id, .. } => UiWidgetSnapshot::Axes {
            fig_id: *fig_id,
            figure: e.ui.uiaxes_figures.get(fig_id).cloned().map(Box::new),
        },
        UiSpecific::ProgressDialog {
            dlg_id,
            title,
            message,
            value,
            indeterminate,
            cancelable,
            cancel_text,
            cancel_requested,
            show_percentage,
            icon,
            ..
        } => UiWidgetSnapshot::ProgressDialog {
            dlg_id: *dlg_id,
            title: title.clone(),
            message: message.clone(),
            value: *value,
            indeterminate: *indeterminate,
            cancelable: *cancelable,
            cancel_text: cancel_text.clone(),
            cancel_requested: *cancel_requested,
            show_percentage: *show_percentage,
            icon: icon.clone(),
        },
    };
    let units = node
        .classic
        .as_ref()
        .map_or_else(|| "pixels".to_string(), |m| m.units.clone());
    UiNodeSnapshot {
        key,
        kind: node.kind,
        parent: node.parent,
        children: node.children.clone(),
        tag: node.tag.clone(),
        enable: node.enable,
        visible: node.visible,
        position: node.position,
        units,
        layout: node.layout,
        font_size: node.font_size,
        background_color: node.background_color,
        tooltip: node.tooltip.clone(),
        is_classic: node.classic.is_some(),
        widget,
    }
}

fn collect_subtree(e: &Engine, root_keys: &[usize], out: &mut BTreeMap<usize, UiNodeSnapshot>) {
    let mut stack: Vec<usize> = root_keys.to_vec();
    while let Some(key) = stack.pop() {
        if out.contains_key(&key) {
            continue;
        }
        if let Some(node) = e.ui.get_key(key) {
            for &ch in &node.children {
                stack.push(ch);
            }
            out.insert(key, snapshot_node(e, key, node));
        }
    }
}

/// Capture an immutable snapshot of all visible UI windows and their components.
pub fn capture_ui_snapshot(engine: &Engine) -> UiSnapshot {
    let mut windows = Vec::new();
    let mut nodes = BTreeMap::new();

    // 1. Modern `uifigure` windows.
    for &fig_key in &engine.ui.figures {
        let Some(fig_node) = engine.ui.get_key(fig_key) else {
            continue;
        };
        if !fig_node.visible {
            continue;
        }
        let (title, color) = match &fig_node.specific {
            UiSpecific::Figure { name, color, .. } => {
                let t = if name.is_empty() {
                    "Figure".to_string()
                } else {
                    name.clone()
                };
                (t, *color)
            }
            _ => ("Figure".to_string(), [0.94, 0.94, 0.94]),
        };
        let mut root_children = Vec::new();
        let mut progress_dialogs = Vec::new();
        for &ch in &fig_node.children {
            if let Some(child) = engine.ui.get_key(ch) {
                if child.kind == UiKind::ProgressDialog {
                    progress_dialogs.push(ch);
                } else {
                    root_children.push(ch);
                }
            }
        }
        collect_subtree(engine, &fig_node.children, &mut nodes);
        windows.push(UiWindowSnapshot {
            fig_key,
            is_classic: false,
            classic_fig_id: None,
            title,
            position: fig_node.position,
            color,
            root_children,
            progress_dialogs,
        });
    }

    // 2. Classic `figure` windows that have UI components (`uicontrol`, `uiaxes`, `uiprogressdlg`)
    //    or active modal sheets (`uialert`, `uiconfirm`).
    for (fig_id, fig) in engine.figures() {
        if !fig.visible {
            continue;
        }
        let fig_key = fig.identity.recovery_key();
        if !engine.ui.has_classic_ui(fig_key) {
            continue;
        }
        let children = engine
            .ui
            .classic_fig_children
            .get(&fig_key)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let mut root_children = Vec::new();
        let mut progress_dialogs = Vec::new();
        for &ch in children {
            if let Some(child) = engine.ui.get_key(ch) {
                if child.kind == UiKind::ProgressDialog {
                    progress_dialogs.push(ch);
                } else {
                    root_children.push(ch);
                }
            }
        }
        collect_subtree(engine, children, &mut nodes);
        let title = if !fig.number_title {
            if fig.name.is_empty() {
                "Figure".to_string()
            } else {
                fig.name.clone()
            }
        } else if fig.name.is_empty() {
            format!("Figure {fig_id}")
        } else {
            format!("Figure {fig_id}: {}", fig.name)
        };
        windows.push(UiWindowSnapshot {
            fig_key,
            is_classic: true,
            classic_fig_id: Some(fig_id),
            title,
            position: fig.position,
            color: [0.94, 0.94, 0.94],
            root_children,
            progress_dialogs,
        });
    }

    UiSnapshot { windows, nodes }
}

/// Dispatch one desktop UI interaction through the exact same callback path as `mrUiFire`.
pub fn dispatch_ui_event(engine: &mut Engine, event: UiEvent, out: &mut Execution) -> Result<()> {
    match event {
        UiEvent::CloseFigure {
            fig_key,
            is_classic,
            classic_fig_id,
        } => {
            if is_classic {
                let res = if let Some(id) = classic_fig_id {
                    engine
                        .call(
                            "close",
                            vec![Value::Matrix(Matrix::scalar(f64::from(id)))],
                            out,
                        )
                        .map(|_| ())
                } else {
                    engine.ui.delete_classic_figure_children(fig_key);
                    Ok(())
                };
                let still_live = engine
                    .figures()
                    .any(|(_, f)| f.identity.recovery_key() == fig_key);
                if !still_live {
                    engine.ui.waiting.remove(&fig_key);
                }
                return res;
            }
            let Some(h) = engine.ui.get_key(fig_key).map(UiNode::handle) else {
                return Ok(());
            };
            close_figure(engine, &h, false, out)
        }
        UiEvent::Press { key } => {
            let Some((h, kind)) = engine.ui.get_key(key).map(|n| (n.handle(), n.kind)) else {
                return Ok(());
            };
            fire_press(engine, key, &h, kind, &[], out)
        }
        UiEvent::TypeText { key, text } => {
            let Some((h, kind)) = engine.ui.get_key(key).map(|n| (n.handle(), n.kind)) else {
                return Ok(());
            };
            fire_type(engine, key, &h, kind, &[Value::Text(text)], out)
        }
        UiEvent::TypeNumeric { key, value } => {
            let Some((h, kind)) = engine.ui.get_key(key).map(|n| (n.handle(), n.kind)) else {
                return Ok(());
            };
            fire_type(
                engine,
                key,
                &h,
                kind,
                &[Value::Matrix(Matrix::scalar(value))],
                out,
            )
        }
        UiEvent::TypeTextArea { key, lines } => {
            let Some((h, kind)) = engine.ui.get_key(key).map(|n| (n.handle(), n.kind)) else {
                return Ok(());
            };
            let mut cells = Vec::with_capacity(lines.len());
            for s in &lines {
                cells.push(Arc::new(CharacterRow::from_text(s, &engine.cancel)?.into()));
            }
            let val = Value::Container(
                CellArray::from_shared(cells.len(), 1, cells, &engine.cancel)?.into(),
            );
            fire_type(engine, key, &h, kind, &[val], out)
        }
        UiEvent::ChooseIndex { key, index } => {
            let Some((h, kind, item_val)) = engine.ui.get_key(key).map(|n| {
                let val = if n.classic.is_some() {
                    Some(Value::Matrix(Matrix::scalar((index + 1) as f64)))
                } else {
                    match &n.specific {
                        UiSpecific::DropDown {
                            items, items_data, ..
                        }
                        | UiSpecific::ListBox {
                            items, items_data, ..
                        } => {
                            if let Some(data) = items_data {
                                data.get(index).cloned()
                            } else {
                                items.get(index).cloned().map(Value::Text)
                            }
                        }
                        UiSpecific::Switch {
                            items, items_data, ..
                        } => {
                            if let Some(data) = items_data {
                                data.get(index).cloned()
                            } else {
                                items.get(index).cloned().map(Value::Text)
                            }
                        }
                        _ => None,
                    }
                };
                (n.handle(), n.kind, val)
            }) else {
                return Ok(());
            };
            if let Some(val) = item_val {
                fire_choose(engine, key, &h, kind, &[val], out)?;
            }
            Ok(())
        }
        UiEvent::ChooseText { key, text } => {
            let Some((h, kind)) = engine.ui.get_key(key).map(|n| (n.handle(), n.kind)) else {
                return Ok(());
            };
            fire_choose(engine, key, &h, kind, &[Value::Text(text)], out)
        }
        UiEvent::ChooseIndices { key, indices } => {
            let Some((h, kind, item_val)) = engine.ui.get_key(key).map(|n| {
                let val = if n.classic.is_some() {
                    indices
                        .first()
                        .map(|&i| Value::Matrix(Matrix::scalar((i + 1) as f64)))
                } else if let UiSpecific::ListBox {
                    items,
                    items_data,
                    multiselect,
                    ..
                } = &n.specific
                {
                    if *multiselect {
                        let mut cells = Vec::with_capacity(indices.len());
                        for &idx in &indices {
                            if let Some(data) = items_data {
                                if let Some(v) = data.get(idx)
                                    && let Ok(arr) = v.clone().array(&engine.cancel)
                                {
                                    cells.push(Arc::new(arr));
                                }
                            } else if let Some(s) = items.get(idx)
                                && let Ok(cr) = CharacterRow::from_text(s, &engine.cancel)
                            {
                                cells.push(Arc::new(cr.into()));
                            }
                        }
                        CellArray::from_shared(1, cells.len(), cells, &engine.cancel)
                            .ok()
                            .map(|c| Value::Container(c.into()))
                    } else if let Some(&idx) = indices.first() {
                        if let Some(data) = items_data {
                            data.get(idx).cloned()
                        } else {
                            items.get(idx).cloned().map(Value::Text)
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                (n.handle(), n.kind, val)
            }) else {
                return Ok(());
            };
            if let Some(val) = item_val {
                fire_choose(engine, key, &h, kind, &[val], out)?;
            }
            Ok(())
        }
        UiEvent::ChooseChild { key } => {
            let Some((h, kind)) = engine.ui.get_key(key).map(|n| (n.handle(), n.kind)) else {
                return Ok(());
            };
            fire_choose(engine, key, &h, kind, &[], out)
        }
        UiEvent::Slide { key, value } => {
            let Some((h, kind)) = engine.ui.get_key(key).map(|n| (n.handle(), n.kind)) else {
                return Ok(());
            };
            fire_slide(
                engine,
                key,
                &h,
                kind,
                &[Value::Matrix(Matrix::scalar(value))],
                out,
            )
        }
        UiEvent::Toggle { key, value } => {
            let Some((h, kind)) = engine.ui.get_key(key).map(|n| (n.handle(), n.kind)) else {
                return Ok(());
            };
            fire_toggle(
                engine,
                key,
                &h,
                kind,
                &[Value::Logical(LogicalMatrix::scalar(value))],
                out,
            )
        }
        UiEvent::EditTableCell {
            key,
            row,
            col,
            value,
        } => {
            let Some((h, kind)) = engine.ui.get_key(key).map(|n| (n.handle(), n.kind)) else {
                return Ok(());
            };
            let idx = Value::Matrix(Matrix::from_column_slice(1, 2, &[row as f64, col as f64])?);
            let val = match value {
                UiTableCell::Numeric(v) => Value::Matrix(Matrix::scalar(v)),
                UiTableCell::Logical(b) => Value::Logical(LogicalMatrix::scalar(b)),
                UiTableCell::Text(s) => Value::Text(s),
            };
            fire_edit(engine, key, &h, kind, &[idx, val], out)
        }
        UiEvent::SelectTableCell { key, row, col } => {
            let Some((h, kind)) = engine.ui.get_key(key).map(|n| (n.handle(), n.kind)) else {
                return Ok(());
            };
            let idx = Value::Matrix(Matrix::from_column_slice(1, 2, &[row as f64, col as f64])?);
            fire_select(engine, key, &h, kind, &[idx], out)
        }
        UiEvent::CancelProgress { key } => {
            let Some(kind) = engine.ui.get_key(key).map(|n| n.kind) else {
                return Ok(());
            };
            fire_cancel(engine, key, kind, &[])
        }
        UiEvent::AlertClosed { id } => {
            engine.ui.pending_alert_figs.remove(&id);
            if let Some((fig_handle, cb)) = engine.ui.pending_alert_close.remove(&id) {
                let src = super::super::graphics_handles::dot_syntax::handle_value(
                    fig_handle,
                    &engine.cancel,
                )?;
                let ev = make_event(engine, src.clone(), "Close", &[])?;
                invoke_callback(engine, "CloseFcn", &cb, src, ev, out)?;
            }
            Ok(())
        }
    }
}
