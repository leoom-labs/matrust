//! Property get/set, validation, and Layout/GridTrack conversions for UI components.
use super::callbacks::{callback_value, parse_callback};
use super::{
    GridSpan, GridTrack, MAX_GRID_TRACKS, MAX_ITEMS, MAX_TEXT_BYTES, TableColumnWidth, TableHeader,
    UiKind, UiLayout, UiSpecific, as_text, make_struct,
};
use crate::engine::{Engine, Value};
use crate::{
    Array, CellArray, CharacterRow, GraphicsArray, GraphicsHandle, GraphicsKind, LogicalMatrix,
    Matrix, Result,
};
use std::sync::Arc;

const COMMON_PROPS: &[&str] = &[
    "type",
    "parent",
    "children",
    "tag",
    "userdata",
    "enable",
    "visible",
    "handlevisibility",
    "position",
    "layout",
    "fontsize",
    "backgroundcolor",
    "tooltip",
];

pub(super) fn is_known_property(kind: UiKind, name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let s = lower.as_str();
    if kind == UiKind::Figure {
        return matches!(
            s,
            "type"
                | "parent"
                | "children"
                | "tag"
                | "userdata"
                | "visible"
                | "handlevisibility"
                | "position"
                | "innerposition"
                | "outerposition"
                | "name"
                | "color"
                | "closerequestfcn"
                | "number"
                | "numbertitle"
        );
    }
    if COMMON_PROPS.contains(&s) {
        return true;
    }
    match kind {
        UiKind::Figure => false,
        UiKind::GridLayout => matches!(
            s,
            "rowheight" | "columnwidth" | "padding" | "rowspacing" | "columnspacing" | "scrollable"
        ),
        UiKind::Panel => matches!(s, "title" | "titleposition" | "bordertype"),
        UiKind::TabGroup => matches!(s, "selectedtab" | "selectionchangedfcn" | "tablocation"),
        UiKind::Tab => matches!(s, "title"),
        UiKind::Button => matches!(s, "text" | "buttonpushedfcn"),
        UiKind::StateButton => matches!(s, "text" | "value" | "valuechangedfcn"),
        UiKind::Label => matches!(
            s,
            "text" | "horizontalalignment" | "verticalalignment" | "wordwrap"
        ),
        UiKind::EditField => matches!(
            s,
            "value" | "editable" | "placeholder" | "valuechangedfcn" | "valuechangingfcn"
        ),
        UiKind::NumericEditField => matches!(
            s,
            "value"
                | "limits"
                | "valuedisplayformat"
                | "editable"
                | "lowerlimitinclusive"
                | "upperlimitinclusive"
                | "roundfractionalvalues"
                | "valuechangedfcn"
                | "valuechangingfcn"
        ),
        UiKind::TextArea => matches!(
            s,
            "value" | "editable" | "wordwrap" | "valuechangedfcn" | "valuechangingfcn"
        ),
        UiKind::DropDown => matches!(
            s,
            "items" | "itemsdata" | "value" | "editable" | "valuechangedfcn"
        ),
        UiKind::ListBox => matches!(
            s,
            "items" | "itemsdata" | "value" | "multiselect" | "valuechangedfcn"
        ),
        UiKind::CheckBox => matches!(s, "text" | "value" | "valuechangedfcn"),
        UiKind::ButtonGroup => matches!(s, "title" | "selectedobject" | "selectionchangedfcn"),
        UiKind::RadioButton => matches!(s, "text" | "value"),
        UiKind::Slider => matches!(
            s,
            "limits"
                | "value"
                | "majorticks"
                | "orientation"
                | "valuechangedfcn"
                | "valuechangingfcn"
        ),
        UiKind::Spinner => matches!(
            s,
            "limits"
                | "step"
                | "value"
                | "valuedisplayformat"
                | "editable"
                | "valuechangedfcn"
                | "valuechangingfcn"
        ),
        UiKind::Switch => matches!(
            s,
            "items" | "itemsdata" | "value" | "orientation" | "valuechangedfcn"
        ),
        UiKind::Table => matches!(
            s,
            "data"
                | "columnname"
                | "rowname"
                | "columneditable"
                | "columnwidth"
                | "celleditcallback"
                | "cellselectioncallback"
        ),
        UiKind::Axes => matches!(
            s,
            "type"
                | "parent"
                | "layout"
                | "position"
                | "innerposition"
                | "outerposition"
                | "tag"
                | "userdata"
                | "visible"
                | "handlevisibility"
        ),
        UiKind::ProgressDialog => matches!(
            s,
            "type"
                | "parent"
                | "title"
                | "message"
                | "value"
                | "indeterminate"
                | "cancelable"
                | "canceltext"
                | "cancelrequested"
                | "showpercentage"
                | "icon"
                | "interpreter"
                | "handlevisibility"
        ),
    }
}

fn unrecognized(e: &mut Engine, kind: UiKind, prop: &str) -> String {
    let class = kind.class_name();
    e.raise(
        format!("Unrecognized property '{prop}' for class '{class}'."),
        "MATLAB:class:InvalidProperty".into(),
    )
}

fn on_off_str(b: bool) -> Value {
    Value::Text(if b { "on" } else { "off" }.into())
}

pub(super) fn parse_on_off(v: &Value, prop: &str) -> Result<bool> {
    if let Some(s) = as_text(v) {
        if s.eq_ignore_ascii_case("on") {
            return Ok(true);
        }
        if s.eq_ignore_ascii_case("off") {
            return Ok(false);
        }
        return Err(format!("{prop} must be 'on' or 'off'."));
    }
    match v {
        Value::Logical(m) => m.as_scalar(),
        Value::Container(Array::Logical(m)) => m.as_scalar(),
        Value::Matrix(m) if m.as_scalar().ok() == Some(1.0) => Ok(true),
        Value::Matrix(m) if m.as_scalar().ok() == Some(0.0) => Ok(false),
        _ => Err(format!("{prop} must be 'on' or 'off'.")),
    }
}

fn parse_logical_scalar(v: &Value, prop: &str) -> Result<bool> {
    match v {
        Value::Logical(m) => m.as_scalar(),
        Value::Container(Array::Logical(m)) => m.as_scalar(),
        Value::Matrix(m) if m.as_scalar().ok() == Some(1.0) => Ok(true),
        Value::Matrix(m) if m.as_scalar().ok() == Some(0.0) => Ok(false),
        _ => Err(format!("{prop} must be a logical scalar.")),
    }
}

pub(super) fn parse_real_scalar(v: &Value, prop: &str) -> Result<f64> {
    match v {
        Value::Matrix(m) if m.len() == 1 => Ok(m.data()[0]),
        Value::Logical(m) if m.len() == 1 => Ok(f64::from(m.data()[0])),
        _ => Err(format!("{prop} must be a real scalar.")),
    }
}

fn parse_real_vec(v: &Value, prop: &str) -> Result<Vec<f64>> {
    match v {
        Value::Matrix(m) if m.rows() <= 1 || m.cols() <= 1 => Ok(m.data().to_vec()),
        Value::Logical(m) if m.rows() <= 1 || m.cols() <= 1 => {
            Ok(m.data().iter().map(|&x| f64::from(x)).collect())
        }
        _ => Err(format!("{prop} must be a real vector.")),
    }
}

pub(super) fn parse_color(e: &Engine, v: &Value, prop: &str) -> Result<[f64; 3]> {
    if let Some(s) = as_text(v) {
        let rgb = crate::engine::plot_properties::color(&Value::Text(s))?;
        return Ok(rgb.map(|c| f64::from(c) / 255.0));
    }
    let vec = parse_real_vec(v, prop)?;
    if vec.len() != 3
        || vec
            .iter()
            .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
    {
        return Err(format!(
            "{prop} must be a 1-by-3 RGB triplet with values in [0, 1] or a color name."
        ));
    }
    let _ = e;
    Ok([vec[0], vec[1], vec[2]])
}

pub(super) fn parse_position(v: &Value, prop: &str) -> Result<[f64; 4]> {
    let vec = parse_real_vec(v, prop)?;
    if vec.len() != 4 || vec.iter().any(|x| !x.is_finite()) || vec[2] < 0.0 || vec[3] < 0.0 {
        return Err(format!(
            "{prop} must be a 4-element finite vector [x y width height] with non-negative width and height."
        ));
    }
    Ok([vec[0], vec[1], vec[2], vec[3]])
}

fn parse_padding(v: &Value) -> Result<[f64; 4]> {
    let vec = parse_real_vec(v, "Padding")?;
    if vec.len() != 4 || vec.iter().any(|x| !x.is_finite() || *x < 0.0) {
        return Err("Padding must be a 4-element non-negative finite vector.".into());
    }
    Ok([vec[0], vec[1], vec[2], vec[3]])
}

pub(super) fn parse_text_bounded(v: &Value, prop: &str) -> Result<String> {
    let s =
        as_text(v).ok_or_else(|| format!("{prop} must be a character vector or string scalar."))?;
    if s.len() > MAX_TEXT_BYTES {
        return Err(format!("{prop} exceeds {MAX_TEXT_BYTES} bytes."));
    }
    Ok(s)
}

pub(super) fn parse_cellstr(e: &Engine, v: &Value, prop: &str) -> Result<Vec<String>> {
    match v {
        Value::Container(Array::Cell(c)) => {
            if c.len() > MAX_ITEMS {
                return Err(format!("{prop} exceeds {MAX_ITEMS} elements."));
            }
            let mut out = Vec::with_capacity(c.len());
            for i in 0..c.len() {
                e.check_cancel()?;
                let item = c.get_linear(i)?;
                let text = match item.as_ref() {
                    Array::Character(ch) if ch.is_text_row() => ch.to_utf8(&e.cancel)?,
                    Array::Str(s) if s.len() == 1 => {
                        s.data()[0].as_deref().unwrap_or("").to_string()
                    }
                    _ => return Err(format!("{prop} must be a cell array of character vectors.")),
                };
                if text.len() > MAX_TEXT_BYTES {
                    return Err(format!("{prop} element exceeds {MAX_TEXT_BYTES} bytes."));
                }
                out.push(text);
            }
            Ok(out)
        }
        Value::Container(Array::Str(s)) => {
            if s.len() > MAX_ITEMS {
                return Err(format!("{prop} exceeds {MAX_ITEMS} elements."));
            }
            let mut out = Vec::with_capacity(s.len());
            for item in s.data() {
                let text = item.as_deref().unwrap_or("").to_string();
                if text.len() > MAX_TEXT_BYTES {
                    return Err(format!("{prop} element exceeds {MAX_TEXT_BYTES} bytes."));
                }
                out.push(text);
            }
            Ok(out)
        }
        _ => {
            if let Some(s) = as_text(v) {
                if s.len() > MAX_TEXT_BYTES {
                    return Err(format!("{prop} exceeds {MAX_TEXT_BYTES} bytes."));
                }
                Ok(vec![s])
            } else {
                Err(format!(
                    "{prop} must be a cell array of character vectors or a string array."
                ))
            }
        }
    }
}

pub(super) fn cellstr_value(e: &Engine, items: &[String], column: bool) -> Result<Value> {
    let mut cells = Vec::with_capacity(items.len());
    for s in items {
        e.check_cancel()?;
        cells.push(Arc::new(CharacterRow::from_text(s, &e.cancel)?.into()));
    }
    let (rows, cols) = if items.is_empty() {
        (1, 0)
    } else if column {
        (items.len(), 1)
    } else {
        (1, items.len())
    };
    Ok(Value::Container(
        CellArray::from_shared(rows, cols, cells, &e.cancel)?.into(),
    ))
}

fn parse_grid_tracks(e: &Engine, v: &Value, prop: &str) -> Result<Vec<GridTrack>> {
    let Value::Container(Array::Cell(c)) = v else {
        return Err(format!(
            "{prop} must be a cell array of positive numbers, 'fit', or weighted strings like '1x'."
        ));
    };
    if c.is_empty() || c.len() > MAX_GRID_TRACKS {
        return Err(format!("{prop} must have 1..={MAX_GRID_TRACKS} elements."));
    }
    let mut tracks = Vec::with_capacity(c.len());
    for i in 0..c.len() {
        e.check_cancel()?;
        let item = c.get_linear(i)?;
        match item.as_ref() {
            Array::Numeric(m) if m.len() == 1 => {
                let x = m.data()[0];
                if !x.is_finite() || x < 0.0 {
                    return Err(format!(
                        "{prop} numeric entries must be non-negative finite numbers."
                    ));
                }
                tracks.push(GridTrack::Pixels(x));
            }
            Array::Character(ch) if ch.is_text_row() => {
                tracks.push(parse_track_str(&ch.to_utf8(&e.cancel)?, prop)?);
            }
            Array::Str(s) if s.len() == 1 => {
                let text = s.data()[0].as_deref().unwrap_or("");
                tracks.push(parse_track_str(text, prop)?);
            }
            _ => {
                return Err(format!(
                    "{prop} entries must be non-negative numbers, 'fit', or weighted strings like '1x'."
                ));
            }
        }
    }
    Ok(tracks)
}

fn parse_track_str(s: &str, prop: &str) -> Result<GridTrack> {
    let trimmed = s.trim();
    if trimmed.eq_ignore_ascii_case("fit") {
        return Ok(GridTrack::Fit);
    }
    if let Some(num) = trimmed
        .strip_suffix('x')
        .or_else(|| trimmed.strip_suffix('X'))
        && let Ok(w) = num.parse::<f64>()
        && w.is_finite()
        && w > 0.0
    {
        return Ok(GridTrack::Weight(w));
    }
    Err(format!(
        "Invalid {prop} entry '{s}'; use a non-negative number, 'fit', or '<weight>x'."
    ))
}

fn tracks_value(e: &Engine, tracks: &[GridTrack]) -> Result<Value> {
    let mut cells = Vec::with_capacity(tracks.len());
    for t in tracks {
        e.check_cancel()?;
        let arr: Array = match t {
            GridTrack::Pixels(px) => Matrix::scalar(*px).into(),
            GridTrack::Fit => CharacterRow::from_text("fit", &e.cancel)?.into(),
            GridTrack::Weight(w) => {
                let s = if w.fract() == 0.0 {
                    format!("{}x", *w as i64)
                } else {
                    format!("{w}x")
                };
                CharacterRow::from_text(&s, &e.cancel)?.into()
            }
        };
        cells.push(Arc::new(arr));
    }
    Ok(Value::Container(
        CellArray::from_shared(1, cells.len(), cells, &e.cancel)?.into(),
    ))
}

fn parse_grid_span(v: &Value, prop: &str) -> Result<GridSpan> {
    let vec = parse_real_vec(v, prop)?;
    let valid =
        |x: f64| x.is_finite() && x >= 1.0 && x.fract() == 0.0 && (x as usize) <= MAX_GRID_TRACKS;
    match vec.as_slice() {
        [a] if valid(*a) => Ok(GridSpan::Single(*a as usize)),
        [a, b] if valid(*a) && valid(*b) && a <= b => Ok(GridSpan::Range(*a as usize, *b as usize)),
        _ => Err(format!(
            "Layout.{prop} must be a positive integer or a 2-element increasing positive integer vector [from to]."
        )),
    }
}

fn span_value(span: GridSpan) -> Result<Value> {
    match span {
        GridSpan::Single(k) => Ok(Value::Matrix(Matrix::scalar(k as f64))),
        GridSpan::Range(a, b) => Ok(Value::Matrix(Matrix::from_column_slice(
            1,
            2,
            &[a as f64, b as f64],
        )?)),
    }
}

fn layout_to_value(e: &Engine, layout: Option<UiLayout>) -> Result<Value> {
    let Some(l) = layout else {
        return Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?));
    };
    make_struct(
        e,
        vec![
            ("Row", span_value(l.row)?),
            ("Column", span_value(l.column)?),
        ],
    )
}

fn parse_layout(v: &Value, current: Option<UiLayout>) -> Result<UiLayout> {
    let Value::Container(Array::Struct(s)) = v else {
        return Err("Layout must be a GridLayoutOptions struct with Row and Column fields.".into());
    };
    if s.len() != 1 {
        return Err("Layout must be a scalar struct.".into());
    }
    let mut out = current.unwrap_or_default();
    for (idx, field) in s.field_names().iter().enumerate() {
        let val = Value::from(s.get(idx, 0)?.as_ref().clone());
        match field.to_ascii_lowercase().as_str() {
            "row" => out.row = parse_grid_span(&val, "Row")?,
            "column" => out.column = parse_grid_span(&val, "Column")?,
            _ => {
                return Err(format!(
                    "Unrecognized property '{field}' for class 'GridLayoutOptions'."
                ));
            }
        }
    }
    Ok(out)
}

fn parse_items_data(e: &Engine, v: &Value) -> Result<Option<Vec<Value>>> {
    match v {
        Value::Matrix(m) if m.is_empty() => Ok(None),
        Value::Matrix(m) if m.rows() == 1 || m.cols() == 1 => {
            if m.len() > MAX_ITEMS {
                return Err(format!("ItemsData exceeds {MAX_ITEMS} elements."));
            }
            Ok(Some(
                m.data()
                    .iter()
                    .map(|&x| Value::Matrix(Matrix::scalar(x)))
                    .collect(),
            ))
        }
        Value::Container(Array::Cell(c)) if c.is_empty() => Ok(None),
        Value::Container(Array::Cell(c)) => {
            if c.len() > MAX_ITEMS {
                return Err(format!("ItemsData exceeds {MAX_ITEMS} elements."));
            }
            let mut out = Vec::with_capacity(c.len());
            for i in 0..c.len() {
                e.check_cancel()?;
                out.push(Value::from(c.get_linear(i)?.as_ref().clone()));
            }
            Ok(Some(out))
        }
        _ => Err("ItemsData must be empty, a numeric vector, or a cell array.".into()),
    }
}

fn items_data_value(e: &Engine, data: &Option<Vec<Value>>) -> Result<Value> {
    let Some(items) = data else {
        return Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?));
    };
    if items
        .iter()
        .all(|v| matches!(v, Value::Matrix(m) if m.len() == 1))
    {
        let nums: Vec<f64> = items
            .iter()
            .map(|v| match v {
                Value::Matrix(m) => m.data()[0],
                _ => 0.0,
            })
            .collect();
        return Ok(Value::Matrix(Matrix::from_column_vec(1, nums.len(), nums)?));
    }
    let mut cells = Vec::with_capacity(items.len());
    for v in items {
        e.check_cancel()?;
        cells.push(Arc::new(v.clone().array(&e.cancel)?));
    }
    Ok(Value::Container(
        CellArray::from_shared(1, cells.len(), cells, &e.cancel)?.into(),
    ))
}

pub(super) fn values_equal(a: &Value, b: &Value) -> bool {
    if let (Some(sa), Some(sb)) = (as_text(a), as_text(b)) {
        return sa == sb;
    }
    match (a, b) {
        (Value::Matrix(ma), Value::Matrix(mb)) => {
            ma.rows() == mb.rows() && ma.cols() == mb.cols() && ma.data() == mb.data()
        }
        (Value::Logical(la), Value::Logical(lb)) => {
            la.rows() == lb.rows() && la.cols() == lb.cols() && la.data() == lb.data()
        }
        _ => false,
    }
}

pub(super) fn validate_dropdown_value(
    items: &[String],
    items_data: &Option<Vec<Value>>,
    editable: bool,
    v: &Value,
) -> Result<Value> {
    if let Some(data) = items_data {
        if let Some(matched) = data.iter().find(|d| values_equal(d, v)) {
            return Ok(matched.clone());
        }
        if editable && let Some(s) = as_text(v) {
            return Ok(Value::Text(s));
        }
        return Err("Value must be an element of ItemsData.".into());
    }
    let s = as_text(v).ok_or("Value must be an element of Items.")?;
    if !editable && !items.iter().any(|it| it == &s) {
        return Err("Value must be an element of Items.".into());
    }
    Ok(Value::Text(s))
}

pub(super) fn validate_listbox_value(
    e: &Engine,
    items: &[String],
    items_data: &Option<Vec<Value>>,
    multiselect: bool,
    v: &Value,
) -> Result<Value> {
    // Empty selection `{}` or `[]`
    if matches!(v, Value::Matrix(m) if m.is_empty())
        || matches!(v, Value::Container(Array::Cell(c)) if c.is_empty())
    {
        return Ok(Value::Container(
            CellArray::from_shared(1, 0, Vec::new(), &e.cancel)?.into(),
        ));
    }
    if !multiselect {
        if let Some(data) = items_data {
            if let Some(matched) = data.iter().find(|d| values_equal(d, v)) {
                return Ok(matched.clone());
            }
            return Err("Value must be an element of ItemsData.".into());
        }
        let s = as_text(v).ok_or("Value must be an element of Items.")?;
        if !items.iter().any(|it| it == &s) {
            return Err("Value must be an element of Items.".into());
        }
        return Ok(Value::Text(s));
    }
    // Multiselect == 'on'
    if let Some(data) = items_data {
        let mut chosen = Vec::new();
        match v {
            Value::Matrix(m) => {
                for &x in m.data() {
                    let cand = Value::Matrix(Matrix::scalar(x));
                    let matched = data
                        .iter()
                        .find(|d| values_equal(d, &cand))
                        .ok_or("Value must be an element of ItemsData.")?;
                    chosen.push(matched.clone());
                }
            }
            Value::Container(Array::Cell(c)) => {
                for i in 0..c.len() {
                    let cand = Value::from(c.get_linear(i)?.as_ref().clone());
                    let matched = data
                        .iter()
                        .find(|d| values_equal(d, &cand))
                        .ok_or("Value must be an element of ItemsData.")?;
                    chosen.push(matched.clone());
                }
            }
            _ => {
                let matched = data
                    .iter()
                    .find(|d| values_equal(d, v))
                    .ok_or("Value must be an element of ItemsData.")?;
                chosen.push(matched.clone());
            }
        }
        return items_data_value(e, &Some(chosen));
    }
    let chosen = parse_cellstr(e, v, "Value")?;
    for s in &chosen {
        if !items.iter().any(|it| it == s) {
            return Err("Value must be an element of Items.".into());
        }
    }
    cellstr_value(e, &chosen, false)
}

pub(super) fn validate_numeric_limits(
    val: f64,
    limits: [f64; 2],
    low_inc: bool,
    high_inc: bool,
) -> Result<()> {
    if !val.is_finite() {
        return Err("Value must be a finite real scalar.".into());
    }
    let low_ok = if low_inc {
        val >= limits[0]
    } else {
        val > limits[0]
    };
    let high_ok = if high_inc {
        val <= limits[1]
    } else {
        val < limits[1]
    };
    if !low_ok || !high_ok {
        return Err("Value must be within the range specified by Limits.".into());
    }
    Ok(())
}

fn auto_major_ticks(limits: [f64; 2]) -> Vec<f64> {
    let step = (limits[1] - limits[0]) / 5.0;
    (0..=5).map(|i| limits[0] + step * (i as f64)).collect()
}

pub(super) fn get_property(e: &mut Engine, h: &GraphicsHandle, args: &[Value]) -> Result<Value> {
    let key = h.object.recovery_key();
    let Some(node) = e.ui.get_key(key) else {
        return Err("Invalid or deleted object.".into());
    };
    if args.is_empty() {
        return Err("get requires a property name for UI components.".into());
    }
    if args.len() != 1 {
        return Err("get expects one property name.".into());
    }
    let prop_raw = as_text(&args[0]).ok_or("Graphics property requires character text")?;
    let prop = prop_raw.to_ascii_lowercase();
    if node.classic.is_some() {
        return super::classic::get_classic_property(e, key, &prop_raw);
    }
    let kind = node.kind;
    if !is_known_property(kind, &prop) {
        return Err(unrecognized(e, kind, &prop_raw));
    }
    let handle_val = |gh: GraphicsHandle| -> Result<Value> {
        Ok(Value::Container(
            GraphicsArray::scalar(gh, &e.cancel)?.into(),
        ))
    };
    // Common properties
    match prop.as_str() {
        "type" => return Ok(Value::Text(kind.type_name().into())),
        "parent" => {
            if let Some(pk) = node.parent {
                let parent = e.ui.get_key(pk).ok_or("Parent object is no longer live.")?;
                return handle_val(parent.handle());
            }
            if let Some(ref fig_id) = node.classic_parent {
                return handle_val(GraphicsHandle::figure(fig_id));
            }
            return handle_val(GraphicsHandle::root());
        }
        "children" => {
            let in_callback = !e.ui.callback_stack.is_empty();
            let mut handles = Vec::with_capacity(node.children.len());
            for &ck in &node.children {
                if let Some(child) = e.ui.get_key(ck)
                    && (e.root.show_hidden_handles
                        || child.handle_visibility.is_visible(in_callback))
                {
                    handles.push(Arc::new(child.handle()));
                }
            }
            let n = handles.len();
            let cols = if n == 0 { 0 } else { 1 };
            return Ok(Value::Container(
                GraphicsArray::from_shared(n, cols, handles, &e.cancel)?.into(),
            ));
        }
        "tag" => return Ok(Value::Text(node.tag.clone())),
        "userdata" => {
            return Ok(node.user_data.clone().unwrap_or_else(|| {
                Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new()).unwrap())
            }));
        }
        "enable" if kind != UiKind::Figure => return Ok(on_off_str(node.enable)),
        "visible" => return Ok(on_off_str(node.visible)),
        "handlevisibility" => return Ok(Value::Text(node.handle_visibility.as_str().into())),
        "position" | "innerposition" | "outerposition" => {
            return Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                4,
                &node.position,
            )?));
        }
        "layout" if kind != UiKind::Figure => return layout_to_value(e, node.layout),
        "fontsize" if kind != UiKind::Figure => {
            return Ok(Value::Matrix(Matrix::scalar(node.font_size)));
        }
        "backgroundcolor" if kind != UiKind::Figure => {
            return Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                3,
                &node.background_color,
            )?));
        }
        "tooltip" if kind != UiKind::Figure => return Ok(Value::Text(node.tooltip.clone())),
        _ => {}
    }

    // Specific properties
    match &node.specific {
        UiSpecific::Figure {
            name,
            color,
            close_request_fcn,
            ..
        } => match prop.as_str() {
            "name" => Ok(Value::Text(name.clone())),
            "color" => Ok(Value::Matrix(Matrix::from_column_slice(1, 3, color)?)),
            "closerequestfcn" => Ok(callback_value(close_request_fcn)),
            "number" => Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?)),
            "numbertitle" => Ok(Value::Text("off".into())),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::GridLayout {
            row_height,
            column_width,
            padding,
            row_spacing,
            column_spacing,
            scrollable,
        } => match prop.as_str() {
            "rowheight" => tracks_value(e, row_height),
            "columnwidth" => tracks_value(e, column_width),
            "padding" => Ok(Value::Matrix(Matrix::from_column_slice(1, 4, padding)?)),
            "rowspacing" => Ok(Value::Matrix(Matrix::scalar(*row_spacing))),
            "columnspacing" => Ok(Value::Matrix(Matrix::scalar(*column_spacing))),
            "scrollable" => Ok(on_off_str(*scrollable)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::Panel {
            title,
            title_position,
            border_type,
        } => match prop.as_str() {
            "title" => Ok(Value::Text(title.clone())),
            "titleposition" => Ok(Value::Text(title_position.clone())),
            "bordertype" => Ok(Value::Text(border_type.clone())),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::TabGroup {
            selected_tab,
            selection_changed_fcn,
            tab_location,
        } => match prop.as_str() {
            "selectedtab" => match selected_tab.and_then(|k| e.ui.get_key(k)) {
                Some(tab) => handle_val(tab.handle()),
                None => Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?)),
            },
            "selectionchangedfcn" => Ok(callback_value(selection_changed_fcn)),
            "tablocation" => Ok(Value::Text(tab_location.clone())),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::Tab { title } => match prop.as_str() {
            "title" => Ok(Value::Text(title.clone())),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::Button {
            text,
            button_pushed_fcn,
        } => match prop.as_str() {
            "text" => Ok(Value::Text(text.clone())),
            "buttonpushedfcn" => Ok(callback_value(button_pushed_fcn)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::StateButton {
            text,
            value,
            value_changed_fcn,
        } => match prop.as_str() {
            "text" => Ok(Value::Text(text.clone())),
            "value" => Ok(Value::Logical(LogicalMatrix::scalar(*value))),
            "valuechangedfcn" => Ok(callback_value(value_changed_fcn)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::Label {
            text,
            horizontal_alignment,
            vertical_alignment,
            word_wrap,
        } => match prop.as_str() {
            "text" => Ok(Value::Text(text.clone())),
            "horizontalalignment" => Ok(Value::Text(horizontal_alignment.clone())),
            "verticalalignment" => Ok(Value::Text(vertical_alignment.clone())),
            "wordwrap" => Ok(on_off_str(*word_wrap)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::EditField {
            value,
            editable,
            placeholder,
            value_changed_fcn,
            value_changing_fcn,
        } => match prop.as_str() {
            "value" => Ok(Value::Text(value.clone())),
            "editable" => Ok(on_off_str(*editable)),
            "placeholder" => Ok(Value::Text(placeholder.clone())),
            "valuechangedfcn" => Ok(callback_value(value_changed_fcn)),
            "valuechangingfcn" => Ok(callback_value(value_changing_fcn)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::NumericEditField {
            value,
            limits,
            value_display_format,
            editable,
            lower_limit_inclusive,
            upper_limit_inclusive,
            round_fractional_values,
            value_changed_fcn,
            value_changing_fcn,
        } => match prop.as_str() {
            "value" => Ok(Value::Matrix(Matrix::scalar(*value))),
            "limits" => Ok(Value::Matrix(Matrix::from_column_slice(1, 2, limits)?)),
            "valuedisplayformat" => Ok(Value::Text(value_display_format.clone())),
            "editable" => Ok(on_off_str(*editable)),
            "lowerlimitinclusive" => Ok(on_off_str(*lower_limit_inclusive)),
            "upperlimitinclusive" => Ok(on_off_str(*upper_limit_inclusive)),
            "roundfractionalvalues" => Ok(on_off_str(*round_fractional_values)),
            "valuechangedfcn" => Ok(callback_value(value_changed_fcn)),
            "valuechangingfcn" => Ok(callback_value(value_changing_fcn)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::TextArea {
            value,
            editable,
            word_wrap,
            value_changed_fcn,
            value_changing_fcn,
        } => match prop.as_str() {
            "value" => cellstr_value(e, value, true),
            "editable" => Ok(on_off_str(*editable)),
            "wordwrap" => Ok(on_off_str(*word_wrap)),
            "valuechangedfcn" => Ok(callback_value(value_changed_fcn)),
            "valuechangingfcn" => Ok(callback_value(value_changing_fcn)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::DropDown {
            items,
            items_data,
            value,
            editable,
            value_changed_fcn,
        } => match prop.as_str() {
            "items" => cellstr_value(e, items, false),
            "itemsdata" => items_data_value(e, items_data),
            "value" => Ok(value.clone()),
            "editable" => Ok(on_off_str(*editable)),
            "valuechangedfcn" => Ok(callback_value(value_changed_fcn)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::ListBox {
            items,
            items_data,
            value,
            multiselect,
            value_changed_fcn,
        } => match prop.as_str() {
            "items" => cellstr_value(e, items, false),
            "itemsdata" => items_data_value(e, items_data),
            "value" => Ok(value.clone()),
            "multiselect" => Ok(on_off_str(*multiselect)),
            "valuechangedfcn" => Ok(callback_value(value_changed_fcn)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::CheckBox {
            text,
            value,
            value_changed_fcn,
        } => match prop.as_str() {
            "text" => Ok(Value::Text(text.clone())),
            "value" => Ok(Value::Logical(LogicalMatrix::scalar(*value))),
            "valuechangedfcn" => Ok(callback_value(value_changed_fcn)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::ButtonGroup {
            title,
            selected_object,
            selection_changed_fcn,
        } => match prop.as_str() {
            "title" => Ok(Value::Text(title.clone())),
            "selectedobject" => match selected_object.and_then(|k| e.ui.get_key(k)) {
                Some(rb) => handle_val(rb.handle()),
                None => Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?)),
            },
            "selectionchangedfcn" => Ok(callback_value(selection_changed_fcn)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::RadioButton { text, value } => match prop.as_str() {
            "text" => Ok(Value::Text(text.clone())),
            "value" => Ok(Value::Logical(LogicalMatrix::scalar(*value))),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::Slider {
            limits,
            value,
            major_ticks,
            orientation,
            value_changed_fcn,
            value_changing_fcn,
            ..
        } => match prop.as_str() {
            "limits" => Ok(Value::Matrix(Matrix::from_column_slice(1, 2, limits)?)),
            "value" => Ok(Value::Matrix(Matrix::scalar(*value))),
            "majorticks" => Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                major_ticks.len(),
                major_ticks,
            )?)),
            "orientation" => Ok(Value::Text(orientation.clone())),
            "valuechangedfcn" => Ok(callback_value(value_changed_fcn)),
            "valuechangingfcn" => Ok(callback_value(value_changing_fcn)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::Spinner {
            limits,
            step,
            value,
            value_display_format,
            editable,
            value_changed_fcn,
            value_changing_fcn,
        } => match prop.as_str() {
            "limits" => Ok(Value::Matrix(Matrix::from_column_slice(1, 2, limits)?)),
            "step" => Ok(Value::Matrix(Matrix::scalar(*step))),
            "value" => Ok(Value::Matrix(Matrix::scalar(*value))),
            "valuedisplayformat" => Ok(Value::Text(value_display_format.clone())),
            "editable" => Ok(on_off_str(*editable)),
            "valuechangedfcn" => Ok(callback_value(value_changed_fcn)),
            "valuechangingfcn" => Ok(callback_value(value_changing_fcn)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::Switch {
            items,
            items_data,
            value,
            orientation,
            value_changed_fcn,
            ..
        } => match prop.as_str() {
            "items" => cellstr_value(e, items, false),
            "itemsdata" => items_data_value(e, items_data),
            "value" => Ok(value.clone()),
            "orientation" => Ok(Value::Text(orientation.clone())),
            "valuechangedfcn" => Ok(callback_value(value_changed_fcn)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::Table {
            data,
            column_name,
            row_name,
            column_editable,
            column_width,
            cell_edit_callback,
            cell_selection_callback,
        } => match prop.as_str() {
            "data" => Ok(data.clone()),
            "columnname" => header_value(e, column_name),
            "rowname" => header_value(e, row_name),
            "columneditable" => Ok(Value::Logical(LogicalMatrix::from_column_vec(
                1,
                column_editable.len(),
                column_editable.iter().map(|&b| u8::from(b)).collect(),
                &e.cancel,
            )?)),
            "columnwidth" => match column_width {
                TableColumnWidth::Auto => Ok(Value::Text("auto".into())),
                TableColumnWidth::Tracks(t) => tracks_value(e, t),
            },
            "celleditcallback" => Ok(callback_value(cell_edit_callback)),
            "cellselectioncallback" => Ok(callback_value(cell_selection_callback)),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
        UiSpecific::Axes { .. } => Err(unrecognized(e, kind, &prop_raw)),
        UiSpecific::ProgressDialog {
            title,
            message,
            value,
            indeterminate,
            cancelable,
            cancel_text,
            cancel_requested,
            show_percentage,
            icon,
            interpreter,
            ..
        } => match prop.as_str() {
            "title" => Ok(Value::Text(title.clone())),
            "message" => Ok(Value::Text(message.clone())),
            "value" => Ok(Value::Matrix(Matrix::scalar(*value))),
            "indeterminate" => Ok(on_off_str(*indeterminate)),
            "cancelable" => Ok(on_off_str(*cancelable)),
            "canceltext" => Ok(Value::Text(cancel_text.clone())),
            "cancelrequested" => Ok(Value::Logical(LogicalMatrix::scalar(*cancel_requested))),
            "showpercentage" => Ok(on_off_str(*show_percentage)),
            "icon" => Ok(Value::Text(icon.clone())),
            "interpreter" => Ok(Value::Text(interpreter.clone())),
            _ => Err(unrecognized(e, kind, &prop_raw)),
        },
    }
}

fn header_value(e: &Engine, header: &TableHeader) -> Result<Value> {
    match header {
        TableHeader::Numbered => Ok(Value::Text("numbered".into())),
        TableHeader::None => Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?)),
        TableHeader::Names(names) => cellstr_value(e, names, true),
    }
}

/// Apply Name-Value pairs at component creation: apply constraint properties
/// (`Limits`, `Items`, `ItemsData`, `Editable`, `Multiselect`) before `Value`.
pub(super) fn apply_creation_pairs(
    e: &mut Engine,
    h: &GraphicsHandle,
    pairs: &[Value],
) -> Result<()> {
    if !pairs.len().is_multiple_of(2) {
        return Err("Properties must be specified as Name, Value pairs.".into());
    }
    let mut deferred = Vec::new();
    for pair in pairs.as_chunks::<2>().0 {
        let name = as_text(&pair[0]).ok_or("Property name must be character text.")?;
        if name.eq_ignore_ascii_case("value") {
            deferred.push(pair);
        } else {
            set_one_property(e, h, &name, &pair[1])?;
        }
    }
    for pair in deferred {
        let name = as_text(&pair[0]).ok_or("Property name must be character text.")?;
        set_one_property(e, h, &name, &pair[1])?;
    }
    Ok(())
}

pub(super) fn set_properties(e: &mut Engine, h: &GraphicsHandle, args: &[Value]) -> Result<()> {
    if args.is_empty() || !args.len().is_multiple_of(2) {
        return Err("set expects Name, Value property pairs.".into());
    }
    for pair in args.as_chunks::<2>().0 {
        let name = as_text(&pair[0]).ok_or("Graphics property requires character text")?;
        set_one_property(e, h, &name, &pair[1])?;
    }
    Ok(())
}

fn set_one_property(e: &mut Engine, h: &GraphicsHandle, prop_raw: &str, val: &Value) -> Result<()> {
    let key = h.object.recovery_key();
    let Some((kind, is_classic)) = e.ui.get_key(key).map(|n| (n.kind, n.classic.is_some())) else {
        return Err("Invalid or deleted object.".into());
    };
    if is_classic {
        return super::classic::set_classic_property(e, key, prop_raw, val);
    }
    let prop = prop_raw.to_ascii_lowercase();
    if !is_known_property(kind, &prop) {
        return Err(unrecognized(e, kind, prop_raw));
    }
    // Read-only properties
    if matches!(
        prop.as_str(),
        "type" | "children" | "number" | "numbertitle" | "cancelrequested"
    ) {
        return Err(format!(
            "Property '{prop_raw}' of class '{}' is read-only.",
            kind.class_name()
        ));
    }
    if prop == "parent" {
        if kind == UiKind::Figure {
            return Err("Property 'Parent' of class 'Figure' is read-only.".into());
        }
        let Value::Container(Array::Graphics(arr)) = val else {
            return Err("Parent must be a UI container handle.".into());
        };
        let ph = arr.scalar_handle()?;
        if ph.kind() != GraphicsKind::UiComponent {
            return Err("Parent must be a UI container handle.".into());
        }
        let new_pk = ph.object.recovery_key();
        if new_pk == key {
            return Err("A component cannot be its own parent.".into());
        }
        let Some(new_parent) = e.ui.get_key(new_pk) else {
            return Err("Invalid or deleted parent object.".into());
        };
        if !new_parent.kind.is_container() {
            return Err("Parent must be a UI container.".into());
        }
        let new_is_grid = new_parent.kind == UiKind::GridLayout;
        let old_pk = e.ui.get_key(key).and_then(|n| n.parent);
        if old_pk != Some(new_pk) {
            if let Some(opk) = old_pk
                && let Some(op) = e.ui.get_mut_key(opk)
            {
                op.children.retain(|&c| c != key);
            }
            if let Some(np) = e.ui.get_mut_key(new_pk) {
                np.children.push(key);
            }
            if let Some(node) = e.ui.get_mut_key(key) {
                node.parent = Some(new_pk);
                node.layout = if new_is_grid {
                    Some(UiLayout::default())
                } else {
                    None
                };
            }
        }
        return Ok(());
    }

    // Handle RadioButton Value side-effect on parent ButtonGroup
    if kind == UiKind::RadioButton && prop == "value" {
        let new_val = parse_logical_scalar(val, "Value")?;
        let parent_key = e.ui.get_key(key).and_then(|n| n.parent);
        if let Some(node) = e.ui.get_mut_key(key)
            && let UiSpecific::RadioButton { value, .. } = &mut node.specific
        {
            *value = new_val;
        }
        if new_val && let Some(pk) = parent_key {
            let siblings: Vec<usize> =
                e.ui.get_key(pk)
                    .map(|p| p.children.clone())
                    .unwrap_or_default();
            for sk in siblings {
                if sk != key
                    && let Some(sib) = e.ui.get_mut_key(sk)
                    && let UiSpecific::RadioButton { value, .. } = &mut sib.specific
                {
                    *value = false;
                }
            }
            if let Some(parent) = e.ui.get_mut_key(pk)
                && let UiSpecific::ButtonGroup {
                    selected_object, ..
                } = &mut parent.specific
            {
                *selected_object = Some(key);
            }
        }
        return Ok(());
    }

    // Handle ButtonGroup SelectedObject
    if kind == UiKind::ButtonGroup && prop == "selectedobject" {
        let Value::Container(Array::Graphics(arr)) = val else {
            return Err("SelectedObject must be a RadioButton child of this ButtonGroup.".into());
        };
        let target = arr.scalar_handle()?;
        let tk = target.object.recovery_key();
        let children =
            e.ui.get_key(key)
                .map(|n| n.children.clone())
                .unwrap_or_default();
        if !children.contains(&tk) {
            return Err("SelectedObject must be a child of this ButtonGroup.".into());
        }
        for ck in children {
            if let Some(child) = e.ui.get_mut_key(ck)
                && let UiSpecific::RadioButton { value, .. } = &mut child.specific
            {
                *value = ck == tk;
            }
        }
        if let Some(node) = e.ui.get_mut_key(key)
            && let UiSpecific::ButtonGroup {
                selected_object, ..
            } = &mut node.specific
        {
            *selected_object = Some(tk);
        }
        return Ok(());
    }

    // Handle TabGroup SelectedTab
    if kind == UiKind::TabGroup && prop == "selectedtab" {
        let Value::Container(Array::Graphics(arr)) = val else {
            return Err("SelectedTab must be a Tab child of this TabGroup.".into());
        };
        let target = arr.scalar_handle()?;
        let tk = target.object.recovery_key();
        let is_child = e.ui.get_key(key).is_some_and(|n| n.children.contains(&tk));
        if !is_child {
            return Err("SelectedTab must be a child of this TabGroup.".into());
        }
        if let Some(node) = e.ui.get_mut_key(key)
            && let UiSpecific::TabGroup { selected_tab, .. } = &mut node.specific
        {
            *selected_tab = Some(tk);
        }
        return Ok(());
    }

    // Common properties
    match prop.as_str() {
        "tag" => {
            let s = parse_text_bounded(val, "Tag")?;
            e.ui.get_mut_key(key).unwrap().tag = s;
            return Ok(());
        }
        "userdata" => {
            e.ui.get_mut_key(key).unwrap().user_data = Some(val.clone());
            return Ok(());
        }
        "enable" if kind != UiKind::Figure => {
            let b = parse_on_off(val, "Enable")?;
            e.ui.get_mut_key(key).unwrap().enable = b;
            return Ok(());
        }
        "visible" => {
            let b = parse_on_off(val, "Visible")?;
            e.ui.get_mut_key(key).unwrap().visible = b;
            return Ok(());
        }
        "handlevisibility" => {
            let s = parse_text_bounded(val, "HandleVisibility")?;
            let hv = super::HandleVisibility::parse(&s)?;
            e.ui.get_mut_key(key).unwrap().handle_visibility = hv;
            return Ok(());
        }
        "position" | "innerposition" | "outerposition" => {
            let p = parse_position(val, prop_raw)?;
            e.ui.get_mut_key(key).unwrap().position = p;
            return Ok(());
        }
        "layout" if kind != UiKind::Figure => {
            let parent_is_grid =
                e.ui.get_key(key)
                    .and_then(|n| n.parent)
                    .and_then(|pk| e.ui.get_key(pk))
                    .is_some_and(|p| p.kind == UiKind::GridLayout);
            if !parent_is_grid {
                return Err(
                    "Layout can only be set when the component's parent is a GridLayout.".into(),
                );
            }
            let current = e.ui.get_key(key).unwrap().layout;
            let parsed = parse_layout(val, current)?;
            e.ui.get_mut_key(key).unwrap().layout = Some(parsed);
            return Ok(());
        }
        "fontsize" if kind != UiKind::Figure => {
            let fs = parse_real_scalar(val, "FontSize")?;
            if !fs.is_finite() || fs <= 0.0 || fs > 500.0 {
                return Err("FontSize must be a positive finite scalar.".into());
            }
            e.ui.get_mut_key(key).unwrap().font_size = fs;
            return Ok(());
        }
        "backgroundcolor" if kind != UiKind::Figure => {
            let rgb = parse_color(e, val, "BackgroundColor")?;
            e.ui.get_mut_key(key).unwrap().background_color = rgb;
            return Ok(());
        }
        "tooltip" if kind != UiKind::Figure => {
            let s = parse_text_bounded(val, "Tooltip")?;
            e.ui.get_mut_key(key).unwrap().tooltip = s;
            return Ok(());
        }
        _ => {}
    }

    // Pre-parse values that need `&Engine` before taking `&mut UiNode`
    let parsed_tracks =
        if kind == UiKind::GridLayout && matches!(prop.as_str(), "rowheight" | "columnwidth") {
            Some(parse_grid_tracks(e, val, prop_raw)?)
        } else {
            None
        };
    let parsed_color = if kind == UiKind::Figure && prop == "color" {
        Some(parse_color(e, val, "Color")?)
    } else {
        None
    };
    let parsed_cb = if prop.ends_with("fcn") || prop.ends_with("callback") {
        Some(parse_callback(e, val)?)
    } else {
        None
    };
    let parsed_cellstr = if (matches!(kind, UiKind::DropDown | UiKind::ListBox | UiKind::Switch)
        && prop == "items")
        || (kind == UiKind::TextArea && prop == "value")
    {
        Some(parse_cellstr(e, val, prop_raw)?)
    } else {
        None
    };
    let parsed_items_data = if matches!(kind, UiKind::DropDown | UiKind::ListBox | UiKind::Switch)
        && prop == "itemsdata"
    {
        Some(parse_items_data(e, val)?)
    } else {
        None
    };
    let parsed_listbox_val = if kind == UiKind::ListBox && prop == "value" {
        let node = e.ui.get_key(key).unwrap();
        if let UiSpecific::ListBox {
            items,
            items_data,
            multiselect,
            ..
        } = &node.specific
        {
            Some(validate_listbox_value(
                e,
                items,
                items_data,
                *multiselect,
                val,
            )?)
        } else {
            None
        }
    } else {
        None
    };
    let parsed_table_header =
        if kind == UiKind::Table && matches!(prop.as_str(), "columnname" | "rowname") {
            Some(parse_table_header(e, val, prop_raw)?)
        } else {
            None
        };
    let parsed_table_width = if kind == UiKind::Table && prop == "columnwidth" {
        Some(
            if as_text(val).is_some_and(|s| s.eq_ignore_ascii_case("auto")) {
                TableColumnWidth::Auto
            } else {
                TableColumnWidth::Tracks(parse_grid_tracks(e, val, "ColumnWidth")?)
            },
        )
    } else {
        None
    };

    let node = e.ui.get_mut_key(key).unwrap();
    match &mut node.specific {
        UiSpecific::Figure {
            name,
            color,
            close_request_fcn,
            ..
        } => match prop.as_str() {
            "name" => *name = parse_text_bounded(val, "Name")?,
            "color" => *color = parsed_color.unwrap(),
            "closerequestfcn" => *close_request_fcn = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::GridLayout {
            row_height,
            column_width,
            padding,
            row_spacing,
            column_spacing,
            scrollable,
        } => match prop.as_str() {
            "rowheight" => *row_height = parsed_tracks.unwrap(),
            "columnwidth" => *column_width = parsed_tracks.unwrap(),
            "padding" => *padding = parse_padding(val)?,
            "rowspacing" => {
                let s = parse_real_scalar(val, "RowSpacing")?;
                if !s.is_finite() || s < 0.0 {
                    return Err("RowSpacing must be a non-negative finite scalar.".into());
                }
                *row_spacing = s;
            }
            "columnspacing" => {
                let s = parse_real_scalar(val, "ColumnSpacing")?;
                if !s.is_finite() || s < 0.0 {
                    return Err("ColumnSpacing must be a non-negative finite scalar.".into());
                }
                *column_spacing = s;
            }
            "scrollable" => *scrollable = parse_on_off(val, "Scrollable")?,
            _ => unreachable!(),
        },
        UiSpecific::Panel {
            title,
            title_position,
            border_type,
        } => match prop.as_str() {
            "title" => *title = parse_text_bounded(val, "Title")?,
            "titleposition" => {
                let s = parse_text_bounded(val, "TitlePosition")?.to_ascii_lowercase();
                if !matches!(
                    s.as_str(),
                    "lefttop"
                        | "centertop"
                        | "righttop"
                        | "leftbottom"
                        | "centerbottom"
                        | "rightbottom"
                ) {
                    return Err(format!("Invalid TitlePosition '{s}'."));
                }
                *title_position = s;
            }
            "bordertype" => {
                let s = parse_text_bounded(val, "BorderType")?.to_ascii_lowercase();
                if !matches!(s.as_str(), "line" | "none") {
                    return Err("BorderType must be 'line' or 'none'.".into());
                }
                *border_type = s;
            }
            _ => unreachable!(),
        },
        UiSpecific::TabGroup {
            selection_changed_fcn,
            tab_location,
            ..
        } => match prop.as_str() {
            "selectionchangedfcn" => *selection_changed_fcn = parsed_cb.unwrap(),
            "tablocation" => {
                let s = parse_text_bounded(val, "TabLocation")?.to_ascii_lowercase();
                if !matches!(s.as_str(), "top" | "bottom" | "left" | "right") {
                    return Err("TabLocation must be 'top', 'bottom', 'left', or 'right'.".into());
                }
                *tab_location = s;
            }
            _ => unreachable!(),
        },
        UiSpecific::Tab { title } => {
            *title = parse_text_bounded(val, "Title")?;
        }
        UiSpecific::Button {
            text,
            button_pushed_fcn,
        } => match prop.as_str() {
            "text" => *text = parse_text_bounded(val, "Text")?,
            "buttonpushedfcn" => *button_pushed_fcn = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::StateButton {
            text,
            value,
            value_changed_fcn,
        } => match prop.as_str() {
            "text" => *text = parse_text_bounded(val, "Text")?,
            "value" => *value = parse_logical_scalar(val, "Value")?,
            "valuechangedfcn" => *value_changed_fcn = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::Label {
            text,
            horizontal_alignment,
            vertical_alignment,
            word_wrap,
        } => match prop.as_str() {
            "text" => *text = parse_text_bounded(val, "Text")?,
            "horizontalalignment" => {
                let s = parse_text_bounded(val, "HorizontalAlignment")?.to_ascii_lowercase();
                if !matches!(s.as_str(), "left" | "center" | "right") {
                    return Err("HorizontalAlignment must be 'left', 'center', or 'right'.".into());
                }
                *horizontal_alignment = s;
            }
            "verticalalignment" => {
                let s = parse_text_bounded(val, "VerticalAlignment")?.to_ascii_lowercase();
                if !matches!(s.as_str(), "top" | "center" | "bottom") {
                    return Err("VerticalAlignment must be 'top', 'center', or 'bottom'.".into());
                }
                *vertical_alignment = s;
            }
            "wordwrap" => *word_wrap = parse_on_off(val, "WordWrap")?,
            _ => unreachable!(),
        },
        UiSpecific::EditField {
            value,
            editable,
            placeholder,
            value_changed_fcn,
            value_changing_fcn,
        } => match prop.as_str() {
            "value" => *value = parse_text_bounded(val, "Value")?,
            "editable" => *editable = parse_on_off(val, "Editable")?,
            "placeholder" => *placeholder = parse_text_bounded(val, "Placeholder")?,
            "valuechangedfcn" => *value_changed_fcn = parsed_cb.unwrap(),
            "valuechangingfcn" => *value_changing_fcn = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::NumericEditField {
            value,
            limits,
            value_display_format,
            editable,
            lower_limit_inclusive,
            upper_limit_inclusive,
            round_fractional_values,
            value_changed_fcn,
            value_changing_fcn,
        } => match prop.as_str() {
            "value" => {
                let mut v = parse_real_scalar(val, "Value")?;
                if *round_fractional_values {
                    v = v.round();
                }
                validate_numeric_limits(
                    v,
                    *limits,
                    *lower_limit_inclusive,
                    *upper_limit_inclusive,
                )?;
                *value = v;
            }
            "limits" => {
                let lim = parse_real_vec(val, "Limits")?;
                if lim.len() != 2 || lim[0].is_nan() || lim[1].is_nan() || lim[0] > lim[1] {
                    return Err("Limits must be a 2-element increasing numeric vector.".into());
                }
                *limits = [lim[0], lim[1]];
                if *value < limits[0] {
                    *value = limits[0];
                } else if *value > limits[1] {
                    *value = limits[1];
                }
            }
            "valuedisplayformat" => {
                *value_display_format = parse_text_bounded(val, "ValueDisplayFormat")?;
            }
            "editable" => *editable = parse_on_off(val, "Editable")?,
            "lowerlimitinclusive" => {
                *lower_limit_inclusive = parse_on_off(val, "LowerLimitInclusive")?
            }
            "upperlimitinclusive" => {
                *upper_limit_inclusive = parse_on_off(val, "UpperLimitInclusive")?
            }
            "roundfractionalvalues" => {
                *round_fractional_values = parse_on_off(val, "RoundFractionalValues")?;
            }
            "valuechangedfcn" => *value_changed_fcn = parsed_cb.unwrap(),
            "valuechangingfcn" => *value_changing_fcn = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::TextArea {
            value,
            editable,
            word_wrap,
            value_changed_fcn,
            value_changing_fcn,
        } => match prop.as_str() {
            "value" => *value = parsed_cellstr.unwrap(),
            "editable" => *editable = parse_on_off(val, "Editable")?,
            "wordwrap" => *word_wrap = parse_on_off(val, "WordWrap")?,
            "valuechangedfcn" => *value_changed_fcn = parsed_cb.unwrap(),
            "valuechangingfcn" => *value_changing_fcn = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::DropDown {
            items,
            items_data,
            value,
            editable,
            value_changed_fcn,
        } => match prop.as_str() {
            "items" => {
                let new_items = parsed_cellstr.unwrap();
                *items = new_items;
                if items_data.is_none()
                    && !*editable
                    && as_text(value).is_none_or(|cur| !items.contains(&cur))
                {
                    *value = Value::Text(items.first().cloned().unwrap_or_default());
                }
            }
            "itemsdata" => {
                *items_data = parsed_items_data.unwrap();
                if let Some(data) = items_data {
                    if !data.iter().any(|d| values_equal(d, value))
                        && let Some(first) = data.first()
                    {
                        *value = first.clone();
                    }
                } else if as_text(value).is_none_or(|cur| !items.contains(&cur)) {
                    *value = Value::Text(items.first().cloned().unwrap_or_default());
                }
            }
            "value" => {
                *value = validate_dropdown_value(items, items_data, *editable, val)?;
            }
            "editable" => *editable = parse_on_off(val, "Editable")?,
            "valuechangedfcn" => *value_changed_fcn = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::ListBox {
            items,
            items_data,
            value,
            multiselect,
            value_changed_fcn,
        } => match prop.as_str() {
            "items" => {
                *items = parsed_cellstr.unwrap();
                if items_data.is_none()
                    && !*multiselect
                    && as_text(value).is_none_or(|cur| !items.contains(&cur))
                {
                    *value = Value::Text(items.first().cloned().unwrap_or_default());
                }
            }
            "itemsdata" => {
                *items_data = parsed_items_data.unwrap();
                if let Some(data) = items_data
                    && !*multiselect
                    && !data.iter().any(|d| values_equal(d, value))
                    && let Some(first) = data.first()
                {
                    *value = first.clone();
                }
            }
            "value" => *value = parsed_listbox_val.unwrap(),
            "multiselect" => *multiselect = parse_on_off(val, "Multiselect")?,
            "valuechangedfcn" => *value_changed_fcn = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::CheckBox {
            text,
            value,
            value_changed_fcn,
        } => match prop.as_str() {
            "text" => *text = parse_text_bounded(val, "Text")?,
            "value" => *value = parse_logical_scalar(val, "Value")?,
            "valuechangedfcn" => *value_changed_fcn = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::ButtonGroup {
            title,
            selection_changed_fcn,
            ..
        } => match prop.as_str() {
            "title" => *title = parse_text_bounded(val, "Title")?,
            "selectionchangedfcn" => *selection_changed_fcn = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::RadioButton { text, .. } => {
            *text = parse_text_bounded(val, "Text")?;
        }
        UiSpecific::Slider {
            limits,
            value,
            major_ticks,
            major_ticks_manual,
            orientation,
            value_changed_fcn,
            value_changing_fcn,
        } => match prop.as_str() {
            "limits" => {
                let lim = parse_real_vec(val, "Limits")?;
                if lim.len() != 2 || !lim[0].is_finite() || !lim[1].is_finite() || lim[0] >= lim[1]
                {
                    return Err(
                        "Limits must be a 2-element finite strictly increasing vector.".into(),
                    );
                }
                *limits = [lim[0], lim[1]];
                if *value < limits[0] {
                    *value = limits[0];
                } else if *value > limits[1] {
                    *value = limits[1];
                }
                if !*major_ticks_manual {
                    *major_ticks = auto_major_ticks(*limits);
                }
            }
            "value" => {
                let v = parse_real_scalar(val, "Value")?;
                validate_numeric_limits(v, *limits, true, true)?;
                *value = v;
            }
            "majorticks" => {
                let ticks = parse_real_vec(val, "MajorTicks")?;
                if ticks.len() > MAX_ITEMS || ticks.iter().any(|x| !x.is_finite()) {
                    return Err("MajorTicks must be a finite real vector.".into());
                }
                *major_ticks = ticks;
                *major_ticks_manual = true;
            }
            "orientation" => {
                let s = parse_text_bounded(val, "Orientation")?.to_ascii_lowercase();
                if !matches!(s.as_str(), "horizontal" | "vertical") {
                    return Err("Orientation must be 'horizontal' or 'vertical'.".into());
                }
                *orientation = s;
            }
            "valuechangedfcn" => *value_changed_fcn = parsed_cb.unwrap(),
            "valuechangingfcn" => *value_changing_fcn = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::Spinner {
            limits,
            step,
            value,
            value_display_format,
            editable,
            value_changed_fcn,
            value_changing_fcn,
        } => match prop.as_str() {
            "limits" => {
                let lim = parse_real_vec(val, "Limits")?;
                if lim.len() != 2 || lim[0].is_nan() || lim[1].is_nan() || lim[0] > lim[1] {
                    return Err("Limits must be a 2-element increasing numeric vector.".into());
                }
                *limits = [lim[0], lim[1]];
                if *value < limits[0] {
                    *value = limits[0];
                } else if *value > limits[1] {
                    *value = limits[1];
                }
            }
            "step" => {
                let s = parse_real_scalar(val, "Step")?;
                if !s.is_finite() || s <= 0.0 {
                    return Err("Step must be a positive finite scalar.".into());
                }
                *step = s;
            }
            "value" => {
                let v = parse_real_scalar(val, "Value")?;
                validate_numeric_limits(v, *limits, true, true)?;
                *value = v;
            }
            "valuedisplayformat" => {
                *value_display_format = parse_text_bounded(val, "ValueDisplayFormat")?;
            }
            "editable" => *editable = parse_on_off(val, "Editable")?,
            "valuechangedfcn" => *value_changed_fcn = parsed_cb.unwrap(),
            "valuechangingfcn" => *value_changing_fcn = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::Switch {
            items,
            items_data,
            value,
            orientation,
            value_changed_fcn,
            ..
        } => match prop.as_str() {
            "items" => {
                let new_items = parsed_cellstr.unwrap();
                if new_items.len() != 2 {
                    return Err(
                        "Switch Items must be a 2-element cell array of character vectors.".into(),
                    );
                }
                *items = [new_items[0].clone(), new_items[1].clone()];
                if items_data.is_none()
                    && as_text(value).is_none_or(|cur| cur != items[0] && cur != items[1])
                {
                    *value = Value::Text(items[0].clone());
                }
            }
            "itemsdata" => {
                let data = parsed_items_data.unwrap();
                if let Some(ref d) = data
                    && d.len() != 2
                {
                    return Err("Switch ItemsData must be empty or have 2 elements.".into());
                }
                *items_data = data;
                if let Some(d) = items_data
                    && !d.iter().any(|x| values_equal(x, value))
                {
                    *value = d[0].clone();
                }
            }
            "value" => {
                *value = validate_dropdown_value(items, items_data, false, val)?;
            }
            "orientation" => {
                let s = parse_text_bounded(val, "Orientation")?.to_ascii_lowercase();
                if !matches!(s.as_str(), "horizontal" | "vertical") {
                    return Err("Orientation must be 'horizontal' or 'vertical'.".into());
                }
                *orientation = s;
            }
            "valuechangedfcn" => *value_changed_fcn = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::Table {
            data,
            column_name,
            row_name,
            column_editable,
            column_width,
            cell_edit_callback,
            cell_selection_callback,
        } => match prop.as_str() {
            "data" => match val {
                Value::Matrix(_)
                | Value::Logical(_)
                | Value::Container(
                    Array::Cell(_) | Array::Table(_) | Array::Logical(_) | Array::Numeric(_),
                ) => {
                    *data = val.clone();
                }
                _ => {
                    return Err(
                        "Table Data must be a numeric array, logical array, cell array, or table."
                            .into(),
                    );
                }
            },
            "columnname" => *column_name = parsed_table_header.unwrap(),
            "rowname" => *row_name = parsed_table_header.unwrap(),
            "columneditable" => {
                *column_editable = parse_logical_vec(val, "ColumnEditable")?;
            }
            "columnwidth" => *column_width = parsed_table_width.unwrap(),
            "celleditcallback" => *cell_edit_callback = parsed_cb.unwrap(),
            "cellselectioncallback" => *cell_selection_callback = parsed_cb.unwrap(),
            _ => unreachable!(),
        },
        UiSpecific::Axes { .. } => unreachable!(),
        UiSpecific::ProgressDialog {
            dlg_id,
            title,
            message,
            value,
            indeterminate,
            cancelable,
            cancel_text,
            show_percentage,
            icon,
            interpreter,
            headless,
            ..
        } => {
            let mut sync = None;
            match prop.as_str() {
                "title" => {
                    *title = parse_text_bounded(val, "Title")?;
                    sync = Some((*dlg_id, title.clone(), message.clone(), *value, *headless));
                }
                "message" => {
                    *message = parse_text_bounded(val, "Message")?;
                    sync = Some((*dlg_id, title.clone(), message.clone(), *value, *headless));
                }
                "value" => {
                    let v = parse_real_scalar(val, "Value")?;
                    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                        return Err("uiprogressdlg Value must be a scalar in [0, 1].".into());
                    }
                    *value = v;
                    sync = Some((*dlg_id, title.clone(), message.clone(), *value, *headless));
                }
                "indeterminate" => *indeterminate = parse_on_off(val, "Indeterminate")?,
                "cancelable" => *cancelable = parse_on_off(val, "Cancelable")?,
                "canceltext" => *cancel_text = parse_text_bounded(val, "CancelText")?,
                "showpercentage" => *show_percentage = parse_on_off(val, "ShowPercentage")?,
                "icon" => *icon = parse_text_bounded(val, "Icon")?.to_ascii_lowercase(),
                "interpreter" => {
                    let s = parse_text_bounded(val, "Interpreter")?.to_ascii_lowercase();
                    if !matches!(s.as_str(), "none" | "tex" | "latex") {
                        return Err("Interpreter must be 'none', 'tex', or 'latex'.".into());
                    }
                    *interpreter = s;
                }
                _ => unreachable!(),
            }
            if let Some((id, t, m, v, hl)) = sync {
                let _ = node;
                super::dialogs_ui::sync_progress_host(e, id, t, &m, v, hl)?;
            }
        }
    }
    Ok(())
}

fn parse_table_header(e: &Engine, v: &Value, prop: &str) -> Result<TableHeader> {
    if matches!(v, Value::Matrix(m) if m.is_empty())
        || matches!(v, Value::Container(Array::Cell(c)) if c.is_empty())
    {
        return Ok(TableHeader::None);
    }
    if let Some(s) = as_text(v) {
        if s.eq_ignore_ascii_case("numbered") {
            return Ok(TableHeader::Numbered);
        }
        if s.is_empty() {
            return Ok(TableHeader::None);
        }
        return Ok(TableHeader::Names(vec![s]));
    }
    Ok(TableHeader::Names(parse_cellstr(e, v, prop)?))
}

fn parse_logical_vec(v: &Value, prop: &str) -> Result<Vec<bool>> {
    match v {
        Value::Logical(m) if m.rows() <= 1 || m.cols() <= 1 => {
            Ok(m.data().iter().map(|&x| x != 0).collect())
        }
        Value::Container(Array::Logical(m)) if m.rows() <= 1 || m.cols() <= 1 => {
            Ok(m.data().iter().map(|&x| x != 0).collect())
        }
        Value::Matrix(m)
            if (m.rows() <= 1 || m.cols() <= 1)
                && m.data().iter().all(|&x| x == 0.0 || x == 1.0) =>
        {
            Ok(m.data().iter().map(|&x| x != 0.0).collect())
        }
        _ => Err(format!("{prop} must be a logical scalar or vector.")),
    }
}
