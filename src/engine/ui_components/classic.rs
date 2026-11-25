//! Classic figure + `uicontrol`, `gcbo`, `gcbf`, and `guidata`.
//!
//! `uicontrol` objects are stored as `UiNode`s in `UiState` with `classic: Some(ClassicMeta)`
//! and mapped onto the same `UiKind` variants as modern 9a1 components so `mrUiFire` works
//! uniformly across both classic and modern UIs.
use super::callbacks::{callback_value, parse_callback};
use super::props::{
    cellstr_value, parse_cellstr, parse_color, parse_on_off, parse_position, parse_real_scalar,
    parse_text_bounded, validate_numeric_limits,
};
use super::{ClassicMeta, UiKind, UiNode, UiSpecific, as_text, default_specific};
use crate::engine::{Engine, Value};
use crate::{Array, GraphicsArray, GraphicsHandle, GraphicsKind, Matrix, ObjectIdentity, Result};

const CLASSIC_STYLES: &[(&str, UiKind)] = &[
    ("pushbutton", UiKind::Button),
    ("togglebutton", UiKind::StateButton),
    ("edit", UiKind::EditField),
    ("text", UiKind::Label),
    ("slider", UiKind::Slider),
    ("checkbox", UiKind::CheckBox),
    ("radiobutton", UiKind::RadioButton),
    ("popupmenu", UiKind::DropDown),
    ("listbox", UiKind::ListBox),
];

pub(super) fn style_to_kind(style: &str) -> Option<UiKind> {
    let lower = style.to_ascii_lowercase();
    CLASSIC_STYLES
        .iter()
        .find(|(s, _)| *s == lower)
        .map(|(_, k)| *k)
}

impl ClassicMeta {
    pub(super) fn new(style: &str) -> Self {
        let lower = style.to_ascii_lowercase();
        let default_val = match lower.as_str() {
            "popupmenu" | "listbox" => 1.0,
            _ => 0.0,
        };
        let default_align = match lower.as_str() {
            "pushbutton" | "togglebutton" | "text" => "center",
            _ => "left",
        };
        Self {
            style: lower,
            units: "pixels".into(),
            min: 0.0,
            max: 1.0,
            value_num: default_val,
            foreground_color: [0.0, 0.0, 0.0],
            horizontal_alignment: default_align.into(),
            callback: None,
            string_raw: Value::Text(String::new()),
            enable_mode: "on".into(),
        }
    }
}

pub(super) fn is_classic_property(prop: &str) -> bool {
    matches!(
        prop,
        "type"
            | "style"
            | "string"
            | "value"
            | "min"
            | "max"
            | "position"
            | "units"
            | "callback"
            | "enable"
            | "visible"
            | "tag"
            | "userdata"
            | "backgroundcolor"
            | "foregroundcolor"
            | "fontsize"
            | "horizontalalignment"
            | "parent"
            | "children"
            | "tooltip"
            | "tooltipstring"
            | "handlevisibility"
    )
}

/// Parse a `uicontrol` `String` value into list items (supports cellstr, `'|'`-delimited char,
/// or single char/string).
pub(super) fn parse_classic_items(e: &Engine, v: &Value) -> Result<Vec<String>> {
    if let Some(s) = as_text(v) {
        if s.is_empty() {
            return Ok(Vec::new());
        }
        return Ok(s.split('|').map(str::to_owned).collect());
    }
    parse_cellstr(e, v, "String")
}

/// Create a classic `uicontrol`: `uicontrol(parent, Name, Value, ...)` or `uicontrol(Name, Value, ...)`.
pub(super) fn create_uicontrol(e: &mut Engine, args: &[Value]) -> Result<Value> {
    let mut ui_parent: Option<usize> = None;
    let mut classic_parent: Option<ObjectIdentity> = None;
    let mut rest = args;

    if let Some(Value::Container(Array::Graphics(arr))) = args.first() {
        let h = arr.scalar_handle()?;
        match h.kind() {
            GraphicsKind::Figure => {
                let located = e.locate_graphics(h)?;
                let fig = e.graphics_figure(located.figure)?;
                classic_parent = Some(fig.identity.clone());
            }
            GraphicsKind::UiComponent => {
                let pk = h.object.recovery_key();
                let Some(node) = e.ui.get_key(pk) else {
                    return Err("Invalid or deleted parent object.".into());
                };
                if !node.kind.is_container() {
                    return Err(format!(
                        "Cannot add uicontrol to component of type '{}'.",
                        node.kind.type_name()
                    ));
                }
                ui_parent = Some(pk);
            }
            _ => {
                return Err("uicontrol Parent must be a figure or UI container handle.".into());
            }
        }
        rest = &args[1..];
    }

    if !rest.len().is_multiple_of(2) {
        return Err("uicontrol expects Name, Value property pairs.".into());
    }

    // Pre-scan for 'Style' and 'Parent' so the node is constructed with the right UiKind and parent.
    let mut style = "pushbutton".to_string();
    let mut filtered_pairs: Vec<(&Value, &Value)> = Vec::with_capacity(rest.len() / 2);
    for pair in rest.as_chunks::<2>().0 {
        let name = as_text(&pair[0]).ok_or("Property name must be character text.")?;
        let lower = name.to_ascii_lowercase();
        if lower == "style" {
            let s = parse_text_bounded(&pair[1], "Style")?.to_ascii_lowercase();
            if style_to_kind(&s).is_none() {
                return Err(format!(
                    "Unsupported uicontrol Style '{s}'; use pushbutton, togglebutton, edit, text, slider, checkbox, radiobutton, popupmenu, or listbox."
                ));
            }
            style = s;
        } else if lower == "parent" {
            let Value::Container(Array::Graphics(arr)) = &pair[1] else {
                return Err("uicontrol Parent must be a figure or UI container handle.".into());
            };
            let h = arr.scalar_handle()?;
            match h.kind() {
                GraphicsKind::Figure => {
                    let located = e.locate_graphics(h)?;
                    let fig = e.graphics_figure(located.figure)?;
                    classic_parent = Some(fig.identity.clone());
                    ui_parent = None;
                }
                GraphicsKind::UiComponent => {
                    let pk = h.object.recovery_key();
                    let Some(node) = e.ui.get_key(pk) else {
                        return Err("Invalid or deleted parent object.".into());
                    };
                    if !node.kind.is_container() {
                        return Err(format!(
                            "Cannot add uicontrol to component of type '{}'.",
                            node.kind.type_name()
                        ));
                    }
                    ui_parent = Some(pk);
                    classic_parent = None;
                }
                _ => {
                    return Err("uicontrol Parent must be a figure or UI container handle.".into());
                }
            }
        } else {
            filtered_pairs.push((&pair[0], &pair[1]));
        }
    }

    // Default parent: current classic figure (creating one if none exists, like gcf).
    if ui_parent.is_none() && classic_parent.is_none() {
        let fig_id = e.ensure_current_classic_figure()?;
        classic_parent = Some(fig_id);
    }

    let kind = style_to_kind(&style).unwrap();
    let mut specific = default_specific(kind);
    // Align initial specific fields with classic defaults (empty string, [0 1] slider, etc.).
    match &mut specific {
        UiSpecific::Button { text, .. }
        | UiSpecific::StateButton { text, .. }
        | UiSpecific::Label { text, .. }
        | UiSpecific::CheckBox { text, .. }
        | UiSpecific::RadioButton { text, .. } => text.clear(),
        UiSpecific::Slider {
            limits,
            value,
            major_ticks,
            ..
        } => {
            *limits = [0.0, 1.0];
            *value = 0.0;
            *major_ticks = vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
        }
        UiSpecific::DropDown { items, value, .. } | UiSpecific::ListBox { items, value, .. } => {
            items.clear();
            *value = Value::Text(String::new());
        }
        _ => {}
    }

    let mut node = UiNode::new(kind, ui_parent, specific);
    node.position = [20.0, 20.0, 60.0, 20.0];
    node.classic = Some(ClassicMeta::new(&style));
    node.classic_parent = classic_parent.clone();

    let key = e.ui.insert_node(node)?;
    if let Some(ref fig_id) = classic_parent {
        e.ui.classic_fig_children
            .entry(fig_id.recovery_key())
            .or_default()
            .push(key);
    }

    let handle = e.ui.get_key(key).unwrap().handle();
    // Apply non-Value properties first (so Min, Max, String are set before Value).
    let mut deferred_value: Option<Value> = None;
    for (k_val, v_val) in filtered_pairs {
        let name = as_text(k_val).unwrap();
        if name.eq_ignore_ascii_case("value") {
            deferred_value = Some(v_val.clone());
        } else if let Err(err) = set_classic_property(e, key, &name, v_val) {
            e.ui.delete_subtree(key);
            return Err(err);
        }
    }
    if let Some(val) = deferred_value
        && let Err(err) = set_classic_property(e, key, "Value", &val)
    {
        e.ui.delete_subtree(key);
        return Err(err);
    }

    super::super::graphics_handles::dot_syntax::handle_value(handle, &e.cancel)
}

pub(super) fn get_classic_property(e: &mut Engine, key: usize, prop_raw: &str) -> Result<Value> {
    let Some(node) = e.ui.get_key(key) else {
        return Err("Invalid or deleted object.".into());
    };
    let meta = node.classic.as_ref().expect("classic uicontrol");
    let prop = prop_raw.to_ascii_lowercase();
    match prop.as_str() {
        "type" => Ok(Value::Text("uicontrol".into())),
        "style" => Ok(Value::Text(meta.style.clone())),
        "string" => Ok(meta.string_raw.clone()),
        "value" => Ok(Value::Matrix(Matrix::scalar(meta.value_num))),
        "min" => Ok(Value::Matrix(Matrix::scalar(meta.min))),
        "max" => Ok(Value::Matrix(Matrix::scalar(meta.max))),
        "position" => Ok(Value::Matrix(Matrix::from_column_vec(
            1,
            4,
            node.position.to_vec(),
        )?)),
        "units" => Ok(Value::Text(meta.units.clone())),
        "callback" => Ok(callback_value(&meta.callback)),
        "enable" => Ok(Value::Text(meta.enable_mode.clone())),
        "visible" => Ok(Value::Text(if node.visible { "on" } else { "off" }.into())),
        "handlevisibility" => Ok(Value::Text(node.handle_visibility.as_str().into())),
        "tag" => Ok(Value::Text(node.tag.clone())),
        "userdata" => Ok(node
            .user_data
            .clone()
            .unwrap_or_else(|| Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new()).unwrap()))),
        "backgroundcolor" => Ok(Value::Matrix(Matrix::from_column_vec(
            1,
            3,
            node.background_color.to_vec(),
        )?)),
        "foregroundcolor" => Ok(Value::Matrix(Matrix::from_column_vec(
            1,
            3,
            meta.foreground_color.to_vec(),
        )?)),
        "fontsize" => Ok(Value::Matrix(Matrix::scalar(node.font_size))),
        "horizontalalignment" => Ok(Value::Text(meta.horizontal_alignment.clone())),
        "tooltip" | "tooltipstring" => Ok(Value::Text(node.tooltip.clone())),
        "children" => Ok(Value::Container(
            GraphicsArray::from_shared(0, 1, Vec::new(), &e.cancel)?.into(),
        )),
        "parent" => {
            if let Some(pk) = node.parent
                && let Some(p) = e.ui.get_key(pk)
            {
                return super::super::graphics_handles::dot_syntax::handle_value(
                    p.handle(),
                    &e.cancel,
                );
            }
            if let Some(ref fig_id) = node.classic_parent {
                return super::super::graphics_handles::dot_syntax::handle_value(
                    GraphicsHandle::figure(fig_id),
                    &e.cancel,
                );
            }
            Ok(Value::Container(
                GraphicsArray::from_shared(0, 1, Vec::new(), &e.cancel)?.into(),
            ))
        }
        _ => Err(e.raise(
            format!("Unrecognized property '{prop_raw}' for class 'UIControl'."),
            "MATLAB:class:InvalidProperty".into(),
        )),
    }
}

pub(super) fn set_classic_property(
    e: &mut Engine,
    key: usize,
    prop_raw: &str,
    val: &Value,
) -> Result<()> {
    let prop = prop_raw.to_ascii_lowercase();
    if !is_classic_property(&prop) {
        return Err(e.raise(
            format!("Unrecognized property '{prop_raw}' for class 'UIControl'."),
            "MATLAB:class:InvalidProperty".into(),
        ));
    }
    if matches!(prop.as_str(), "type" | "children") {
        return Err(format!(
            "Property '{prop_raw}' of class 'UIControl' is read-only."
        ));
    }
    if prop == "parent" {
        return Err("Reparenting uicontrol after creation is not supported yet.".into());
    }

    match prop.as_str() {
        "style" => {
            let s = parse_text_bounded(val, "Style")?.to_ascii_lowercase();
            let Some(new_kind) = style_to_kind(&s) else {
                return Err(format!("Unsupported uicontrol Style '{s}'."));
            };
            let node = e.ui.get_mut_key(key).ok_or("Invalid or deleted object.")?;
            node.kind = new_kind;
            node.specific = default_specific(new_kind);
            if let Some(meta) = node.classic.as_mut() {
                meta.style = s;
            }
        }
        "string" => {
            let kind = e.ui.get_key(key).ok_or("Invalid or deleted object.")?.kind;
            if matches!(kind, UiKind::DropDown | UiKind::ListBox) {
                let items = parse_classic_items(e, val)?;
                let stored_raw = match val {
                    Value::Container(Array::Cell(_)) => cellstr_value(e, &items, false)?,
                    _ => Value::Text(as_text(val).unwrap_or_else(|| items.join("|"))),
                };
                let node = e.ui.get_mut_key(key).ok_or("Invalid or deleted object.")?;
                if let Some(meta) = node.classic.as_mut() {
                    meta.string_raw = stored_raw;
                    if !items.is_empty()
                        && (meta.value_num < 1.0 || meta.value_num as usize > items.len())
                    {
                        meta.value_num = 1.0;
                    }
                    let sel_idx = (meta.value_num as usize).saturating_sub(1);
                    let sel_text = items.get(sel_idx).cloned().unwrap_or_default();
                    match &mut node.specific {
                        UiSpecific::DropDown {
                            items: it, value, ..
                        }
                        | UiSpecific::ListBox {
                            items: it, value, ..
                        } => {
                            *it = items;
                            *value = Value::Text(sel_text);
                        }
                        _ => {}
                    }
                }
            } else {
                let s = if let Some(t) = as_text(val) {
                    t
                } else {
                    parse_cellstr(e, val, "String")?.join("\n")
                };
                let node = e.ui.get_mut_key(key).ok_or("Invalid or deleted object.")?;
                if let Some(meta) = node.classic.as_mut() {
                    meta.string_raw = Value::Text(s.clone());
                }
                match &mut node.specific {
                    UiSpecific::Button { text, .. }
                    | UiSpecific::StateButton { text, .. }
                    | UiSpecific::Label { text, .. }
                    | UiSpecific::CheckBox { text, .. }
                    | UiSpecific::RadioButton { text, .. } => *text = s,
                    UiSpecific::EditField { value, .. } => *value = s,
                    _ => {}
                }
            }
        }
        "min" => {
            let v = parse_real_scalar(val, "Min")?;
            if !v.is_finite() {
                return Err("Min must be a finite real scalar.".into());
            }
            let node = e.ui.get_mut_key(key).ok_or("Invalid or deleted object.")?;
            if let Some(meta) = node.classic.as_mut() {
                meta.min = v;
                if let UiSpecific::Slider { limits, .. } = &mut node.specific {
                    limits[0] = v;
                }
            }
        }
        "max" => {
            let v = parse_real_scalar(val, "Max")?;
            if !v.is_finite() {
                return Err("Max must be a finite real scalar.".into());
            }
            let node = e.ui.get_mut_key(key).ok_or("Invalid or deleted object.")?;
            if let Some(meta) = node.classic.as_mut() {
                meta.max = v;
                if let UiSpecific::Slider { limits, .. } = &mut node.specific {
                    limits[1] = v;
                }
            }
        }
        "value" => {
            let v = parse_real_scalar(val, "Value")?;
            if !v.is_finite() {
                return Err("Value must be a finite real scalar.".into());
            }
            let node = e.ui.get_mut_key(key).ok_or("Invalid or deleted object.")?;
            let meta = node.classic.as_mut().expect("classic uicontrol");
            match node.kind {
                UiKind::Slider => {
                    validate_numeric_limits(v, [meta.min, meta.max], true, true)?;
                    meta.value_num = v;
                    if let UiSpecific::Slider { value, .. } = &mut node.specific {
                        *value = v;
                    }
                }
                UiKind::DropDown | UiKind::ListBox => {
                    let count = match &node.specific {
                        UiSpecific::DropDown { items, .. } | UiSpecific::ListBox { items, .. } => {
                            items.len()
                        }
                        _ => 0,
                    };
                    if v.fract() != 0.0 || v < 1.0 || (count > 0 && v as usize > count) {
                        return Err(format!(
                            "Value must be an integer index in 1..={}.",
                            count.max(1)
                        ));
                    }
                    meta.value_num = v;
                    let idx = v as usize - 1;
                    match &mut node.specific {
                        UiSpecific::DropDown { items, value, .. }
                        | UiSpecific::ListBox { items, value, .. } => {
                            if let Some(item) = items.get(idx) {
                                *value = Value::Text(item.clone());
                            }
                        }
                        _ => {}
                    }
                }
                UiKind::CheckBox | UiKind::StateButton | UiKind::RadioButton => {
                    meta.value_num = v;
                    let is_on = (v - meta.min).abs() > f64::EPSILON;
                    match &mut node.specific {
                        UiSpecific::CheckBox { value, .. }
                        | UiSpecific::StateButton { value, .. }
                        | UiSpecific::RadioButton { value, .. } => *value = is_on,
                        _ => {}
                    }
                }
                _ => {
                    meta.value_num = v;
                }
            }
        }
        "position" => {
            let p = parse_position(val, "Position")?;
            e.ui.get_mut_key(key)
                .ok_or("Invalid or deleted object.")?
                .position = p;
        }
        "units" => {
            let u = parse_text_bounded(val, "Units")?.to_ascii_lowercase();
            if !matches!(
                u.as_str(),
                "pixels" | "normalized" | "characters" | "points" | "inches" | "centimeters"
            ) {
                return Err(format!(
                    "Unsupported Units '{u}'; use 'pixels' or 'normalized'."
                ));
            }
            e.ui.get_mut_key(key)
                .ok_or("Invalid or deleted object.")?
                .classic
                .as_mut()
                .unwrap()
                .units = u;
        }
        "callback" => {
            let cb = parse_callback(e, val)?;
            e.ui.get_mut_key(key)
                .ok_or("Invalid or deleted object.")?
                .classic
                .as_mut()
                .unwrap()
                .callback = cb;
        }
        "enable" => {
            let mode = if let Some(s) = as_text(val) {
                let lower = s.to_ascii_lowercase();
                if !matches!(lower.as_str(), "on" | "off" | "inactive") {
                    return Err("Enable must be 'on', 'off', or 'inactive'.".into());
                }
                lower
            } else if parse_on_off(val, "Enable")? {
                "on".into()
            } else {
                "off".into()
            };
            let node = e.ui.get_mut_key(key).ok_or("Invalid or deleted object.")?;
            node.enable = mode == "on";
            node.classic.as_mut().unwrap().enable_mode = mode;
        }
        "visible" => {
            let b = parse_on_off(val, "Visible")?;
            e.ui.get_mut_key(key)
                .ok_or("Invalid or deleted object.")?
                .visible = b;
        }
        "handlevisibility" => {
            let s = parse_text_bounded(val, "HandleVisibility")?;
            let hv = super::HandleVisibility::parse(&s)?;
            e.ui.get_mut_key(key)
                .ok_or("Invalid or deleted object.")?
                .handle_visibility = hv;
        }
        "tag" => {
            let s = parse_text_bounded(val, "Tag")?;
            e.ui.get_mut_key(key)
                .ok_or("Invalid or deleted object.")?
                .tag = s;
        }
        "userdata" => {
            e.ui.get_mut_key(key)
                .ok_or("Invalid or deleted object.")?
                .user_data = Some(val.clone());
        }
        "backgroundcolor" => {
            let c = parse_color(e, val, "BackgroundColor")?;
            e.ui.get_mut_key(key)
                .ok_or("Invalid or deleted object.")?
                .background_color = c;
        }
        "foregroundcolor" => {
            let c = parse_color(e, val, "ForegroundColor")?;
            e.ui.get_mut_key(key)
                .ok_or("Invalid or deleted object.")?
                .classic
                .as_mut()
                .unwrap()
                .foreground_color = c;
        }
        "fontsize" => {
            let fs = parse_real_scalar(val, "FontSize")?;
            if !fs.is_finite() || fs <= 0.0 {
                return Err("FontSize must be a positive finite scalar.".into());
            }
            e.ui.get_mut_key(key)
                .ok_or("Invalid or deleted object.")?
                .font_size = fs;
        }
        "horizontalalignment" => {
            let s = parse_text_bounded(val, "HorizontalAlignment")?.to_ascii_lowercase();
            if !matches!(s.as_str(), "left" | "center" | "right") {
                return Err("HorizontalAlignment must be 'left', 'center', or 'right'.".into());
            }
            e.ui.get_mut_key(key)
                .ok_or("Invalid or deleted object.")?
                .classic
                .as_mut()
                .unwrap()
                .horizontal_alignment = s;
        }
        "tooltip" | "tooltipstring" => {
            let s = parse_text_bounded(val, "Tooltip")?;
            e.ui.get_mut_key(key)
                .ok_or("Invalid or deleted object.")?
                .tooltip = s;
        }
        _ => unreachable!(),
    }
    Ok(())
}

/// Resolve any live graphics handle to its enclosing figure `(recovery_key, figure_handle)`.
pub(super) fn enclosing_figure(e: &Engine, h: &GraphicsHandle) -> Result<(usize, GraphicsHandle)> {
    match h.kind() {
        GraphicsKind::UiComponent => {
            let mut cur = h.object.recovery_key();
            for _ in 0..super::MAX_UI_COMPONENTS {
                let Some(node) = e.ui.get_key(cur) else {
                    return Err("Invalid or deleted object.".into());
                };
                if node.kind == UiKind::Figure {
                    return Ok((cur, node.handle()));
                }
                if let Some(ref fig_id) = node.classic_parent {
                    return Ok((fig_id.recovery_key(), GraphicsHandle::figure(fig_id)));
                }
                if let Some(pk) = node.parent {
                    cur = pk;
                } else {
                    return Err("Component has no enclosing figure.".into());
                }
            }
            Err("Component hierarchy cycle detected.".into())
        }
        GraphicsKind::Axes if e.ui.is_uiaxes(&h.object) => {
            let mut cur = h.object.recovery_key();
            for _ in 0..super::MAX_UI_COMPONENTS {
                let Some(node) = e.ui.get_key(cur) else {
                    return Err("Invalid or deleted object.".into());
                };
                if node.kind == UiKind::Figure {
                    return Ok((cur, node.handle()));
                }
                if let Some(ref fig_id) = node.classic_parent {
                    return Ok((fig_id.recovery_key(), GraphicsHandle::figure(fig_id)));
                }
                if let Some(pk) = node.parent {
                    cur = pk;
                } else {
                    return Err("Axes has no enclosing figure.".into());
                }
            }
            Err("Component hierarchy cycle detected.".into())
        }
        _ => {
            let located = e.locate_graphics(h)?;
            if super::is_uiaxes_fig_id(located.figure) {
                // Series/text inside a uiaxes -> walk up from the uiaxes node.
                let ax_id = h.axes.as_ref().ok_or("Invalid or deleted object.")?;
                let ax_h = GraphicsHandle::axes(&h.figure, ax_id);
                return enclosing_figure(e, &ax_h);
            }
            let fig = e.graphics_figure(located.figure)?;
            Ok((
                fig.identity.recovery_key(),
                GraphicsHandle::figure(&fig.identity),
            ))
        }
    }
}

/// `gcbo`: handle of the object whose callback is currently executing, or `[]`.
pub(super) fn call_gcbo(e: &Engine, args: &[Value]) -> Result<Value> {
    if !args.is_empty() {
        return Err("gcbo takes no input arguments.".into());
    }
    if let Some(h) = e.ui.callback_stack.last() {
        super::super::graphics_handles::dot_syntax::handle_value(h.clone(), &e.cancel)
    } else {
        Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?))
    }
}

/// `gcbf`: handle of the figure containing `gcbo`, or `[]`.
pub(super) fn call_gcbf(e: &Engine, args: &[Value]) -> Result<Value> {
    if !args.is_empty() {
        return Err("gcbf takes no input arguments.".into());
    }
    if let Some(h) = e.ui.callback_stack.last()
        && let Ok((_, fig_h)) = enclosing_figure(e, h)
    {
        return super::super::graphics_handles::dot_syntax::handle_value(fig_h, &e.cancel);
    }
    Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?))
}

/// `guidata(h, data)` / `data = guidata(h)`: store or retrieve application data on the figure.
pub(super) fn call_guidata(e: &mut Engine, args: &[Value]) -> Result<Value> {
    if args.is_empty() || args.len() > 2 {
        return Err("Use guidata(h) or guidata(h, data).".into());
    }
    let Value::Container(Array::Graphics(arr)) = &args[0] else {
        return Err("guidata requires a valid graphics handle.".into());
    };
    let h = arr.scalar_handle()?.clone();
    let (fig_key, _) = enclosing_figure(e, &h)?;
    if args.len() == 2 {
        e.ui.guidata.insert(fig_key, args[1].clone());
        Ok(Value::Void)
    } else {
        Ok(e.ui
            .guidata
            .get(&fig_key)
            .cloned()
            .unwrap_or_else(|| Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new()).unwrap())))
    }
}
