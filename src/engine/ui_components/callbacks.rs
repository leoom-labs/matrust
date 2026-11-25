//! Callback storage, invocation, event structs, and test-only `mrUiFire`.
use super::props::{
    parse_cellstr, validate_dropdown_value, validate_listbox_value, validate_numeric_limits,
    values_equal,
};
use super::{UiCallback, UiKind, UiSpecific, as_text, close_figure, make_struct};
use crate::engine::{Engine, Execution, Value};
use crate::table_array::TableArray;
use crate::{
    Array, CharacterRow, FunctionHandle, GraphicsArray, GraphicsHandle, GraphicsKind,
    LogicalMatrix, Matrix, Result,
};
use std::sync::Arc;

pub(super) fn parse_callback(e: &Engine, v: &Value) -> Result<Option<UiCallback>> {
    match v {
        Value::Matrix(m) if m.is_empty() => Ok(None),
        Value::Text(s) | Value::QuotedString(s) if s.is_empty() => Ok(None),
        Value::Container(Array::Character(c)) if c.is_empty() => Ok(None),
        Value::Container(Array::Cell(c)) if c.is_empty() => Ok(None),
        Value::Container(Array::Handle(fh)) => Ok(Some(UiCallback::Handle(fh.clone()))),
        Value::Container(Array::Cell(cell)) => {
            let first = cell.get_linear(0)?;
            match first.as_ref() {
                Array::Handle(_) => Ok(Some(UiCallback::Cell(cell.clone()))),
                Array::Character(ch) if ch.is_text_row() => {
                    let name = ch.to_utf8(&e.cancel)?;
                    if name.trim().is_empty() {
                        return Err("Callback cell array first element must be a function handle or function name.".into());
                    }
                    Ok(Some(UiCallback::Cell(cell.clone())))
                }
                _ => Err("Callback cell array first element must be a function handle.".into()),
            }
        }
        _ => {
            if let Some(s) = as_text(v) {
                if s.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(UiCallback::Text(s)))
                }
            } else {
                Err("Callback must be a function handle, a cell array starting with a function handle, a character vector, or empty.".into())
            }
        }
    }
}

pub(super) fn callback_value(cb: &Option<UiCallback>) -> Value {
    match cb {
        None => Value::Text(String::new()),
        Some(UiCallback::Handle(fh)) => Value::Container(Array::Handle(fh.clone())),
        Some(UiCallback::Cell(c)) => Value::Container(Array::Cell(c.clone())),
        Some(UiCallback::Text(s)) => Value::Text(s.clone()),
    }
}

pub(super) fn make_event(
    e: &Engine,
    src: Value,
    event_name: &str,
    extra: &[(&str, Value)],
) -> Result<Value> {
    let mut fields = Vec::with_capacity(2 + extra.len());
    fields.push(("Source", src));
    fields.push(("EventName", Value::Text(event_name.into())));
    for (k, v) in extra {
        fields.push((*k, v.clone()));
    }
    make_struct(e, fields)
}

/// Invoke a UI callback. Pushes `src` onto `e.ui.callback_stack` for `gcbo`/`gcbf`
/// and guarantees popping on all exit paths. Errors inside the callback are reported
/// as a warning naming the callback property and do NOT abort the caller (except cancellation).
pub(super) fn invoke_callback(
    e: &mut Engine,
    callback_name: &str,
    cb: &UiCallback,
    src: Value,
    event: Value,
    out: &mut Execution,
) -> Result<()> {
    e.check_cancel()?;
    let pushed = if let Value::Container(Array::Graphics(arr)) = &src
        && let Ok(h) = arr.scalar_handle()
    {
        e.ui.callback_stack.push(h.clone());
        true
    } else {
        false
    };
    let run_result: Result<()> = match cb {
        UiCallback::Handle(fh) => e
            .call_handle(fh, vec![src, event], 0, None, out)
            .map(|_| ()),
        UiCallback::Cell(cell) => {
            let first = cell.get_linear(0)?;
            let mut call_args = Vec::with_capacity(cell.len() + 1);
            call_args.push(src);
            call_args.push(event);
            for i in 1..cell.len() {
                call_args.push(Value::from(cell.get_linear(i)?.as_ref().clone()));
            }
            match first.as_ref() {
                Array::Handle(fh) => e.call_handle(fh, call_args, 0, None, out).map(|_| ()),
                Array::Character(ch) if ch.is_text_row() => {
                    let name = ch.to_utf8(&e.cancel)?;
                    let fh = FunctionHandle::global(name.trim());
                    e.call_handle(&fh, call_args, 0, None, out).map(|_| ())
                }
                _ => Err("Invalid callback cell array.".into()),
            }
        }
        UiCallback::Text(code) => e.eval_in_base(code, out),
    };
    if pushed {
        e.ui.callback_stack.pop();
    }
    if let Err(err) = run_result {
        e.check_cancel()?;
        e.clear_raised_error();
        let msg = format!("Error executing {callback_name} callback: {err}");
        let _ = e.lng_warn(&msg, "MATLAB:ui:CallbackError", out);
    }
    Ok(())
}

fn fire_classic_action(
    e: &mut Engine,
    key: usize,
    h: &GraphicsHandle,
    out: &mut Execution,
) -> Result<()> {
    let cb =
        e.ui.get_key(key)
            .and_then(|n| n.classic.as_ref())
            .and_then(|m| m.callback.clone());
    if let Some(cb) = cb {
        let src = handle_val(e, h.clone())?;
        let ev = make_event(e, src.clone(), "Action", &[])?;
        invoke_callback(e, "Callback", &cb, src, ev, out)?;
    }
    Ok(())
}

fn handle_val(e: &Engine, h: GraphicsHandle) -> Result<Value> {
    Ok(Value::Container(
        GraphicsArray::scalar(h, &e.cancel)?.into(),
    ))
}

/// Test-only trigger: `mrUiFire(h, action, args...)`.
pub(super) fn mr_ui_fire(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<()> {
    if args.len() < 2 {
        return Err("mrUiFire requires a component handle and an action name.".into());
    }
    let Value::Container(Array::Graphics(arr)) = &args[0] else {
        return Err("mrUiFire first argument must be a UI component handle.".into());
    };
    let h = arr.scalar_handle()?.clone();
    if h.kind() != GraphicsKind::UiComponent {
        return Err("mrUiFire requires a UI component handle.".into());
    }
    let key = h.object.recovery_key();
    let Some(kind) = e.ui.get_key(key).map(|n| n.kind) else {
        return Err("Invalid or deleted object.".into());
    };
    let action = as_text(&args[1])
        .ok_or("mrUiFire action must be a character vector or string.")?
        .to_ascii_lowercase();
    let rest = &args[2..];

    match action.as_str() {
        "press" => fire_press(e, key, &h, kind, rest, out),
        "type" => fire_type(e, key, &h, kind, rest, out),
        "choose" => fire_choose(e, key, &h, kind, rest, out),
        "slide" => fire_slide(e, key, &h, kind, rest, out),
        "toggle" => fire_toggle(e, key, &h, kind, rest, out),
        "edit" => fire_edit(e, key, &h, kind, rest, out),
        "select" => fire_select(e, key, &h, kind, rest, out),
        "cancel" => fire_cancel(e, key, kind, rest),
        "close" => {
            if !rest.is_empty() {
                return Err("mrUiFire(fig, 'close') takes no extra arguments.".into());
            }
            close_figure(e, &h, false, out)
        }
        _ => Err(format!(
            "mrUiFire: unknown action '{action}'; use 'press', 'type', 'choose', 'slide', 'toggle', 'edit', 'select', 'cancel', or 'close'."
        )),
    }
}

pub(super) fn fire_cancel(e: &mut Engine, key: usize, kind: UiKind, rest: &[Value]) -> Result<()> {
    if !rest.is_empty() {
        return Err("mrUiFire(d, 'cancel') takes no extra arguments.".into());
    }
    if kind != UiKind::ProgressDialog {
        return Err("mrUiFire 'cancel' requires a uiprogressdlg handle.".into());
    }
    if let Some(node) = e.ui.get_mut_key(key)
        && let UiSpecific::ProgressDialog {
            cancel_requested, ..
        } = &mut node.specific
    {
        *cancel_requested = true;
    }
    Ok(())
}

pub(super) fn fire_press(
    e: &mut Engine,
    key: usize,
    h: &GraphicsHandle,
    kind: UiKind,
    rest: &[Value],
    out: &mut Execution,
) -> Result<()> {
    if !rest.is_empty() {
        return Err("mrUiFire(btn, 'press') takes no extra arguments.".into());
    }
    if kind != UiKind::Button {
        return Err("mrUiFire 'press' requires a push uibutton or pushbutton uicontrol.".into());
    }
    if e.ui.get_key(key).is_some_and(|n| n.classic.is_some()) {
        return fire_classic_action(e, key, h, out);
    }
    let cb = match &e.ui.get_key(key).unwrap().specific {
        UiSpecific::Button {
            button_pushed_fcn, ..
        } => button_pushed_fcn.clone(),
        _ => None,
    };
    if let Some(cb) = cb {
        let src = handle_val(e, h.clone())?;
        let ev = make_event(e, src.clone(), "ButtonPushed", &[])?;
        invoke_callback(e, "ButtonPushedFcn", &cb, src, ev, out)?;
    }
    Ok(())
}

pub(super) fn fire_type(
    e: &mut Engine,
    key: usize,
    h: &GraphicsHandle,
    kind: UiKind,
    rest: &[Value],
    out: &mut Execution,
) -> Result<()> {
    let [new_val] = rest else {
        return Err("mrUiFire(h, 'type', value) requires one value argument.".into());
    };
    if kind == UiKind::EditField && e.ui.get_key(key).is_some_and(|n| n.classic.is_some()) {
        let new_s = as_text(new_val).ok_or("Edit uicontrol requires character text.")?;
        if let Some(node) = e.ui.get_mut_key(key) {
            if let Some(meta) = node.classic.as_mut() {
                meta.string_raw = Value::Text(new_s.clone());
            }
            if let UiSpecific::EditField { value, .. } = &mut node.specific {
                *value = new_s;
            }
        }
        return fire_classic_action(e, key, h, out);
    }
    let src = handle_val(e, h.clone())?;
    match kind {
        UiKind::EditField => {
            let new_s = as_text(new_val).ok_or("Text edit field requires character text.")?;
            let (prev, changing_cb, changed_cb) = match &e.ui.get_key(key).unwrap().specific {
                UiSpecific::EditField {
                    value,
                    value_changing_fcn,
                    value_changed_fcn,
                    ..
                } => (
                    Value::Text(value.clone()),
                    value_changing_fcn.clone(),
                    value_changed_fcn.clone(),
                ),
                _ => unreachable!(),
            };
            let new_v = Value::Text(new_s.clone());
            if let Some(cb) = changing_cb {
                let ev = make_event(e, src.clone(), "ValueChanging", &[("Value", new_v.clone())])?;
                invoke_callback(e, "ValueChangingFcn", &cb, src.clone(), ev, out)?;
            }
            if let Some(node) = e.ui.get_mut_key(key)
                && let UiSpecific::EditField { value, .. } = &mut node.specific
            {
                *value = new_s;
            }
            if let Some(cb) = changed_cb {
                let ev = make_event(
                    e,
                    src.clone(),
                    "ValueChanged",
                    &[("Value", new_v), ("PreviousValue", prev)],
                )?;
                invoke_callback(e, "ValueChangedFcn", &cb, src, ev, out)?;
            }
            Ok(())
        }
        UiKind::NumericEditField => {
            let v = match new_val {
                Value::Matrix(m) if m.len() == 1 => m.data()[0],
                Value::Logical(m) if m.len() == 1 => f64::from(m.data()[0]),
                _ => return Err("Numeric edit field requires a real scalar.".into()),
            };
            let (prev, limits, low_inc, high_inc, round, changing_cb, changed_cb) =
                match &e.ui.get_key(key).unwrap().specific {
                    UiSpecific::NumericEditField {
                        value,
                        limits,
                        lower_limit_inclusive,
                        upper_limit_inclusive,
                        round_fractional_values,
                        value_changing_fcn,
                        value_changed_fcn,
                        ..
                    } => (
                        *value,
                        *limits,
                        *lower_limit_inclusive,
                        *upper_limit_inclusive,
                        *round_fractional_values,
                        value_changing_fcn.clone(),
                        value_changed_fcn.clone(),
                    ),
                    _ => unreachable!(),
                };
            let v = if round { v.round() } else { v };
            validate_numeric_limits(v, limits, low_inc, high_inc)?;
            let new_v = Value::Matrix(Matrix::scalar(v));
            if let Some(cb) = changing_cb {
                let ev = make_event(e, src.clone(), "ValueChanging", &[("Value", new_v.clone())])?;
                invoke_callback(e, "ValueChangingFcn", &cb, src.clone(), ev, out)?;
            }
            if let Some(node) = e.ui.get_mut_key(key)
                && let UiSpecific::NumericEditField { value, .. } = &mut node.specific
            {
                *value = v;
            }
            if let Some(cb) = changed_cb {
                let ev = make_event(
                    e,
                    src.clone(),
                    "ValueChanged",
                    &[
                        ("Value", new_v),
                        ("PreviousValue", Value::Matrix(Matrix::scalar(prev))),
                    ],
                )?;
                invoke_callback(e, "ValueChangedFcn", &cb, src, ev, out)?;
            }
            Ok(())
        }
        UiKind::Spinner => {
            let v = match new_val {
                Value::Matrix(m) if m.len() == 1 => m.data()[0],
                _ => return Err("Spinner requires a real scalar.".into()),
            };
            let (prev, limits, changing_cb, changed_cb) = match &e.ui.get_key(key).unwrap().specific
            {
                UiSpecific::Spinner {
                    value,
                    limits,
                    value_changing_fcn,
                    value_changed_fcn,
                    ..
                } => (
                    *value,
                    *limits,
                    value_changing_fcn.clone(),
                    value_changed_fcn.clone(),
                ),
                _ => unreachable!(),
            };
            validate_numeric_limits(v, limits, true, true)?;
            let new_v = Value::Matrix(Matrix::scalar(v));
            if let Some(cb) = changing_cb {
                let ev = make_event(e, src.clone(), "ValueChanging", &[("Value", new_v.clone())])?;
                invoke_callback(e, "ValueChangingFcn", &cb, src.clone(), ev, out)?;
            }
            if let Some(node) = e.ui.get_mut_key(key)
                && let UiSpecific::Spinner { value, .. } = &mut node.specific
            {
                *value = v;
            }
            if let Some(cb) = changed_cb {
                let ev = make_event(
                    e,
                    src.clone(),
                    "ValueChanged",
                    &[
                        ("Value", new_v),
                        ("PreviousValue", Value::Matrix(Matrix::scalar(prev))),
                    ],
                )?;
                invoke_callback(e, "ValueChangedFcn", &cb, src, ev, out)?;
            }
            Ok(())
        }
        UiKind::TextArea => {
            let new_lines = parse_cellstr(e, new_val, "Value")?;
            let (prev_lines, changing_cb, changed_cb) = match &e.ui.get_key(key).unwrap().specific {
                UiSpecific::TextArea {
                    value,
                    value_changing_fcn,
                    value_changed_fcn,
                    ..
                } => (
                    value.clone(),
                    value_changing_fcn.clone(),
                    value_changed_fcn.clone(),
                ),
                _ => unreachable!(),
            };
            let to_cell = |lines: &[String]| -> Result<Value> {
                let mut cells = Vec::with_capacity(lines.len());
                for s in lines {
                    cells.push(Arc::new(CharacterRow::from_text(s, &e.cancel)?.into()));
                }
                Ok(Value::Container(
                    crate::CellArray::from_shared(cells.len(), 1, cells, &e.cancel)?.into(),
                ))
            };
            let new_v = to_cell(&new_lines)?;
            let prev_v = to_cell(&prev_lines)?;
            if let Some(cb) = changing_cb {
                let ev = make_event(e, src.clone(), "ValueChanging", &[("Value", new_v.clone())])?;
                invoke_callback(e, "ValueChangingFcn", &cb, src.clone(), ev, out)?;
            }
            if let Some(node) = e.ui.get_mut_key(key)
                && let UiSpecific::TextArea { value, .. } = &mut node.specific
            {
                *value = new_lines;
            }
            if let Some(cb) = changed_cb {
                let ev = make_event(
                    e,
                    src.clone(),
                    "ValueChanged",
                    &[("Value", new_v), ("PreviousValue", prev_v)],
                )?;
                invoke_callback(e, "ValueChangedFcn", &cb, src, ev, out)?;
            }
            Ok(())
        }
        _ => Err("mrUiFire 'type' requires a uieditfield, uispinner, or uitextarea.".into()),
    }
}

pub(super) fn fire_choose(
    e: &mut Engine,
    key: usize,
    h: &GraphicsHandle,
    kind: UiKind,
    rest: &[Value],
    out: &mut Execution,
) -> Result<()> {
    match kind {
        UiKind::DropDown => {
            let [item] = rest else {
                return Err("mrUiFire(dd, 'choose', item) requires one item argument.".into());
            };
            if e.ui.get_key(key).is_some_and(|n| n.classic.is_some()) {
                return fire_classic_choose(e, key, h, item, out);
            }
            let (validated, prev, cb) = match &e.ui.get_key(key).unwrap().specific {
                UiSpecific::DropDown {
                    items,
                    items_data,
                    value,
                    editable,
                    value_changed_fcn,
                } => (
                    validate_dropdown_value(items, items_data, *editable, item)?,
                    value.clone(),
                    value_changed_fcn.clone(),
                ),
                _ => unreachable!(),
            };
            if let Some(node) = e.ui.get_mut_key(key)
                && let UiSpecific::DropDown { value, .. } = &mut node.specific
            {
                *value = validated.clone();
            }
            if let Some(cb) = cb {
                let src = handle_val(e, h.clone())?;
                let ev = make_event(
                    e,
                    src.clone(),
                    "ValueChanged",
                    &[("Value", validated), ("PreviousValue", prev)],
                )?;
                invoke_callback(e, "ValueChangedFcn", &cb, src, ev, out)?;
            }
            Ok(())
        }
        UiKind::ListBox => {
            let [item] = rest else {
                return Err("mrUiFire(lb, 'choose', item) requires one item argument.".into());
            };
            if e.ui.get_key(key).is_some_and(|n| n.classic.is_some()) {
                return fire_classic_choose(e, key, h, item, out);
            }
            let (validated, prev, cb) = match &e.ui.get_key(key).unwrap().specific {
                UiSpecific::ListBox {
                    items,
                    items_data,
                    value,
                    multiselect,
                    value_changed_fcn,
                } => (
                    validate_listbox_value(e, items, items_data, *multiselect, item)?,
                    value.clone(),
                    value_changed_fcn.clone(),
                ),
                _ => unreachable!(),
            };
            if let Some(node) = e.ui.get_mut_key(key)
                && let UiSpecific::ListBox { value, .. } = &mut node.specific
            {
                *value = validated.clone();
            }
            if let Some(cb) = cb {
                let src = handle_val(e, h.clone())?;
                let ev = make_event(
                    e,
                    src.clone(),
                    "ValueChanged",
                    &[("Value", validated), ("PreviousValue", prev)],
                )?;
                invoke_callback(e, "ValueChangedFcn", &cb, src, ev, out)?;
            }
            Ok(())
        }
        UiKind::Switch => {
            let [item] = rest else {
                return Err("mrUiFire(sw, 'choose', item) requires one item argument.".into());
            };
            let (validated, prev, cb) = match &e.ui.get_key(key).unwrap().specific {
                UiSpecific::Switch {
                    items,
                    items_data,
                    value,
                    value_changed_fcn,
                    ..
                } => (
                    validate_dropdown_value(items, items_data, false, item)?,
                    value.clone(),
                    value_changed_fcn.clone(),
                ),
                _ => unreachable!(),
            };
            if let Some(node) = e.ui.get_mut_key(key)
                && let UiSpecific::Switch { value, .. } = &mut node.specific
            {
                *value = validated.clone();
            }
            if let Some(cb) = cb {
                let src = handle_val(e, h.clone())?;
                let ev = make_event(
                    e,
                    src.clone(),
                    "ValueChanged",
                    &[("Value", validated), ("PreviousValue", prev)],
                )?;
                invoke_callback(e, "ValueChangedFcn", &cb, src, ev, out)?;
            }
            Ok(())
        }
        UiKind::RadioButton => {
            if !rest.is_empty() {
                return Err("mrUiFire(rb, 'choose') takes no extra arguments.".into());
            }
            let pk = e
                .ui
                .get_key(key)
                .and_then(|n| n.parent)
                .ok_or("RadioButton has no parent ButtonGroup.")?;
            select_in_button_group(e, pk, key, out)
        }
        UiKind::ButtonGroup => {
            let [target] = rest else {
                return Err("mrUiFire(bg, 'choose', rb) requires a RadioButton handle or text.".into());
            };
            let target_key = resolve_group_child(e, key, target, UiKind::RadioButton)?;
            select_in_button_group(e, key, target_key, out)
        }
        UiKind::Tab => {
            if !rest.is_empty() {
                return Err("mrUiFire(tab, 'choose') takes no extra arguments.".into());
            }
            let pk = e
                .ui
                .get_key(key)
                .and_then(|n| n.parent)
                .ok_or("Tab has no parent TabGroup.")?;
            select_in_tab_group(e, pk, key, out)
        }
        UiKind::TabGroup => {
            let [target] = rest else {
                return Err("mrUiFire(tg, 'choose', tab) requires a Tab handle or title.".into());
            };
            let target_key = resolve_group_child(e, key, target, UiKind::Tab)?;
            select_in_tab_group(e, key, target_key, out)
        }
        _ => Err(
            "mrUiFire 'choose' requires a uidropdown, uilistbox, uiswitch, uiradiobutton, uibuttongroup, uitab, or uitabgroup."
                .into(),
        ),
    }
}

fn resolve_group_child(
    e: &Engine,
    group_key: usize,
    target: &Value,
    child_kind: UiKind,
) -> Result<usize> {
    let children =
        e.ui.get_key(group_key)
            .map(|n| n.children.clone())
            .unwrap_or_default();
    if let Value::Container(Array::Graphics(arr)) = target {
        let ch = arr.scalar_handle()?;
        let ck = ch.object.recovery_key();
        if children.contains(&ck) {
            return Ok(ck);
        }
        return Err("Specified handle is not a child of this group.".into());
    }
    if let Some(label) = as_text(target) {
        for ck in children {
            if let Some(node) = e.ui.get_key(ck)
                && node.kind == child_kind
            {
                let matches = match &node.specific {
                    UiSpecific::RadioButton { text, .. } => text == &label,
                    UiSpecific::Tab { title } => title == &label,
                    _ => false,
                };
                if matches {
                    return Ok(ck);
                }
            }
        }
        return Err(format!("No child matching '{label}' found in group."));
    }
    Err("Expected a child handle or label text.".into())
}

fn select_in_button_group(
    e: &mut Engine,
    bg_key: usize,
    new_key: usize,
    out: &mut Execution,
) -> Result<()> {
    let (old_key, cb, bg_handle, children) = {
        let bg =
            e.ui.get_key(bg_key)
                .ok_or("ButtonGroup is no longer live.")?;
        let (old, cb) = match &bg.specific {
            UiSpecific::ButtonGroup {
                selected_object,
                selection_changed_fcn,
                ..
            } => (*selected_object, selection_changed_fcn.clone()),
            _ => return Err("Parent is not a ButtonGroup.".into()),
        };
        (old, cb, bg.handle(), bg.children.clone())
    };
    for ck in children {
        if let Some(child) = e.ui.get_mut_key(ck)
            && let UiSpecific::RadioButton { value, .. } = &mut child.specific
        {
            *value = ck == new_key;
        }
    }
    if let Some(bg) = e.ui.get_mut_key(bg_key)
        && let UiSpecific::ButtonGroup {
            selected_object, ..
        } = &mut bg.specific
    {
        *selected_object = Some(new_key);
    }
    if let Some(cb) = cb {
        let src = handle_val(e, bg_handle)?;
        let new_val = handle_val(e, e.ui.get_key(new_key).unwrap().handle())?;
        let old_val = match old_key.and_then(|k| e.ui.get_key(k)) {
            Some(old) => handle_val(e, old.handle())?,
            None => Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?),
        };
        let ev = make_event(
            e,
            src.clone(),
            "SelectionChanged",
            &[("NewValue", new_val), ("OldValue", old_val)],
        )?;
        invoke_callback(e, "SelectionChangedFcn", &cb, src, ev, out)?;
    }
    Ok(())
}

fn select_in_tab_group(
    e: &mut Engine,
    tg_key: usize,
    new_key: usize,
    out: &mut Execution,
) -> Result<()> {
    let (old_key, cb, tg_handle) = {
        let tg = e.ui.get_key(tg_key).ok_or("TabGroup is no longer live.")?;
        let (old, cb) = match &tg.specific {
            UiSpecific::TabGroup {
                selected_tab,
                selection_changed_fcn,
                ..
            } => (*selected_tab, selection_changed_fcn.clone()),
            _ => return Err("Parent is not a TabGroup.".into()),
        };
        (old, cb, tg.handle())
    };
    if let Some(tg) = e.ui.get_mut_key(tg_key)
        && let UiSpecific::TabGroup { selected_tab, .. } = &mut tg.specific
    {
        *selected_tab = Some(new_key);
    }
    if let Some(cb) = cb {
        let src = handle_val(e, tg_handle)?;
        let new_val = handle_val(e, e.ui.get_key(new_key).unwrap().handle())?;
        let old_val = match old_key.and_then(|k| e.ui.get_key(k)) {
            Some(old) => handle_val(e, old.handle())?,
            None => Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?),
        };
        let ev = make_event(
            e,
            src.clone(),
            "SelectionChanged",
            &[("NewValue", new_val), ("OldValue", old_val)],
        )?;
        invoke_callback(e, "SelectionChangedFcn", &cb, src, ev, out)?;
    }
    Ok(())
}

fn fire_classic_choose(
    e: &mut Engine,
    key: usize,
    h: &GraphicsHandle,
    item: &Value,
    out: &mut Execution,
) -> Result<()> {
    let items: Vec<String> = match &e.ui.get_key(key).unwrap().specific {
        UiSpecific::DropDown { items, .. } | UiSpecific::ListBox { items, .. } => items.clone(),
        _ => Vec::new(),
    };
    let idx: usize = if let Some(s) = as_text(item) {
        items
            .iter()
            .position(|it| it == &s)
            .map(|p| p + 1)
            .ok_or_else(|| format!("Item '{s}' not found in uicontrol String."))?
    } else if let Value::Matrix(m) = item
        && m.len() == 1
    {
        let v = m.data()[0];
        if !v.is_finite() || v.fract() != 0.0 || v < 1.0 || v as usize > items.len() {
            return Err(format!(
                "Index must be an integer in 1..={}.",
                items.len().max(1)
            ));
        }
        v as usize
    } else {
        return Err(
            "mrUiFire 'choose' on uicontrol requires an item string or 1-based index.".into(),
        );
    };
    let chosen_text = items.get(idx - 1).cloned().unwrap_or_default();
    if let Some(node) = e.ui.get_mut_key(key) {
        if let Some(meta) = node.classic.as_mut() {
            meta.value_num = idx as f64;
        }
        match &mut node.specific {
            UiSpecific::DropDown { value, .. } | UiSpecific::ListBox { value, .. } => {
                *value = Value::Text(chosen_text);
            }
            _ => {}
        }
    }
    fire_classic_action(e, key, h, out)
}

pub(super) fn fire_slide(
    e: &mut Engine,
    key: usize,
    h: &GraphicsHandle,
    kind: UiKind,
    rest: &[Value],
    out: &mut Execution,
) -> Result<()> {
    if kind != UiKind::Slider {
        return Err("mrUiFire 'slide' requires a uislider or slider uicontrol.".into());
    }
    let [new_val] = rest else {
        return Err("mrUiFire(sld, 'slide', value) requires one numeric value.".into());
    };
    let v = match new_val {
        Value::Matrix(m) if m.len() == 1 => m.data()[0],
        _ => return Err("Slider value must be a real scalar.".into()),
    };
    if e.ui.get_key(key).is_some_and(|n| n.classic.is_some()) {
        let (min, max) = {
            let meta = e.ui.get_key(key).unwrap().classic.as_ref().unwrap();
            (meta.min, meta.max)
        };
        validate_numeric_limits(v, [min, max], true, true)?;
        if let Some(node) = e.ui.get_mut_key(key) {
            if let Some(meta) = node.classic.as_mut() {
                meta.value_num = v;
            }
            if let UiSpecific::Slider { value, .. } = &mut node.specific {
                *value = v;
            }
        }
        return fire_classic_action(e, key, h, out);
    }
    let (prev, limits, changing_cb, changed_cb) = match &e.ui.get_key(key).unwrap().specific {
        UiSpecific::Slider {
            value,
            limits,
            value_changing_fcn,
            value_changed_fcn,
            ..
        } => (
            *value,
            *limits,
            value_changing_fcn.clone(),
            value_changed_fcn.clone(),
        ),
        _ => unreachable!(),
    };
    validate_numeric_limits(v, limits, true, true)?;
    let src = handle_val(e, h.clone())?;
    let new_v = Value::Matrix(Matrix::scalar(v));
    if let Some(cb) = changing_cb {
        let ev = make_event(e, src.clone(), "ValueChanging", &[("Value", new_v.clone())])?;
        invoke_callback(e, "ValueChangingFcn", &cb, src.clone(), ev, out)?;
    }
    if let Some(node) = e.ui.get_mut_key(key)
        && let UiSpecific::Slider { value, .. } = &mut node.specific
    {
        *value = v;
    }
    if let Some(cb) = changed_cb {
        let ev = make_event(
            e,
            src.clone(),
            "ValueChanged",
            &[
                ("Value", new_v),
                ("PreviousValue", Value::Matrix(Matrix::scalar(prev))),
            ],
        )?;
        invoke_callback(e, "ValueChangedFcn", &cb, src, ev, out)?;
    }
    Ok(())
}

pub(super) fn fire_toggle(
    e: &mut Engine,
    key: usize,
    h: &GraphicsHandle,
    kind: UiKind,
    rest: &[Value],
    out: &mut Execution,
) -> Result<()> {
    if rest.len() > 1 {
        return Err("mrUiFire 'toggle' takes at most one optional target value.".into());
    }
    let src = handle_val(e, h.clone())?;
    if matches!(
        kind,
        UiKind::CheckBox | UiKind::StateButton | UiKind::RadioButton
    ) && e.ui.get_key(key).is_some_and(|n| n.classic.is_some())
    {
        let (old_bool, min_v, max_v) = {
            let node = e.ui.get_key(key).unwrap();
            let meta = node.classic.as_ref().unwrap();
            let is_on = (meta.value_num - meta.min).abs() > f64::EPSILON;
            (is_on, meta.min, meta.max)
        };
        let new_bool = match rest.first() {
            None => !old_bool,
            Some(Value::Logical(m)) => m.as_scalar()?,
            Some(Value::Matrix(m)) if m.as_scalar().ok() == Some(1.0) => true,
            Some(Value::Matrix(m)) if m.as_scalar().ok() == Some(0.0) => false,
            _ => return Err("Toggle target value must be logical or 0/1.".into()),
        };
        let new_num = if new_bool { max_v } else { min_v };
        if let Some(node) = e.ui.get_mut_key(key) {
            if let Some(meta) = node.classic.as_mut() {
                meta.value_num = new_num;
            }
            match &mut node.specific {
                UiSpecific::CheckBox { value, .. }
                | UiSpecific::StateButton { value, .. }
                | UiSpecific::RadioButton { value, .. } => *value = new_bool,
                _ => {}
            }
        }
        return fire_classic_action(e, key, h, out);
    }
    match kind {
        UiKind::CheckBox | UiKind::StateButton => {
            let (old, cb) = match &e.ui.get_key(key).unwrap().specific {
                UiSpecific::CheckBox {
                    value,
                    value_changed_fcn,
                    ..
                }
                | UiSpecific::StateButton {
                    value,
                    value_changed_fcn,
                    ..
                } => (*value, value_changed_fcn.clone()),
                _ => unreachable!(),
            };
            let new_bool = match rest.first() {
                None => !old,
                Some(Value::Logical(m)) => m.as_scalar()?,
                Some(Value::Matrix(m)) if m.as_scalar().ok() == Some(1.0) => true,
                Some(Value::Matrix(m)) if m.as_scalar().ok() == Some(0.0) => false,
                _ => return Err("Toggle target value must be logical.".into()),
            };
            if let Some(node) = e.ui.get_mut_key(key) {
                match &mut node.specific {
                    UiSpecific::CheckBox { value, .. } | UiSpecific::StateButton { value, .. } => {
                        *value = new_bool
                    }
                    _ => {}
                }
            }
            if let Some(cb) = cb {
                let ev = make_event(
                    e,
                    src.clone(),
                    "ValueChanged",
                    &[
                        ("Value", Value::Logical(LogicalMatrix::scalar(new_bool))),
                        ("PreviousValue", Value::Logical(LogicalMatrix::scalar(old))),
                    ],
                )?;
                invoke_callback(e, "ValueChangedFcn", &cb, src, ev, out)?;
            }
            Ok(())
        }
        UiKind::Switch => {
            let (next_val, prev, cb) = match &e.ui.get_key(key).unwrap().specific {
                UiSpecific::Switch {
                    items,
                    items_data,
                    value,
                    value_changed_fcn,
                    ..
                } => {
                    let target = if let Some(explicit) = rest.first() {
                        validate_dropdown_value(items, items_data, false, explicit)?
                    } else if let Some(data) = items_data {
                        if values_equal(value, &data[0]) {
                            data[1].clone()
                        } else {
                            data[0].clone()
                        }
                    } else if as_text(value).as_deref() == Some(items[0].as_str()) {
                        Value::Text(items[1].clone())
                    } else {
                        Value::Text(items[0].clone())
                    };
                    (target, value.clone(), value_changed_fcn.clone())
                }
                _ => unreachable!(),
            };
            if let Some(node) = e.ui.get_mut_key(key)
                && let UiSpecific::Switch { value, .. } = &mut node.specific
            {
                *value = next_val.clone();
            }
            if let Some(cb) = cb {
                let ev = make_event(
                    e,
                    src.clone(),
                    "ValueChanged",
                    &[("Value", next_val), ("PreviousValue", prev)],
                )?;
                invoke_callback(e, "ValueChangedFcn", &cb, src, ev, out)?;
            }
            Ok(())
        }
        _ => Err("mrUiFire 'toggle' requires a uicheckbox, state uibutton, or uiswitch.".into()),
    }
}

fn parse_indices(v: &Value) -> Result<(usize, usize)> {
    let Value::Matrix(m) = v else {
        return Err("Cell indices must be a 2-element positive integer vector [row col].".into());
    };
    if m.len() != 2 || (m.rows() != 1 && m.cols() != 1) {
        return Err("Cell indices must be a 2-element positive integer vector [row col].".into());
    }
    let r = m.data()[0];
    let c = m.data()[1];
    if !r.is_finite()
        || !c.is_finite()
        || r < 1.0
        || c < 1.0
        || r.fract() != 0.0
        || c.fract() != 0.0
    {
        return Err("Cell indices must be positive integers.".into());
    }
    Ok((r as usize, c as usize))
}

pub(super) fn fire_edit(
    e: &mut Engine,
    key: usize,
    h: &GraphicsHandle,
    kind: UiKind,
    rest: &[Value],
    out: &mut Execution,
) -> Result<()> {
    if kind != UiKind::Table {
        return Err("mrUiFire 'edit' requires a uitable.".into());
    }
    let [idx_val, edit_val] = rest else {
        return Err(
            "mrUiFire(tbl, 'edit', [r c], newValue) requires indices and a new value.".into(),
        );
    };
    let (r, c) = parse_indices(idx_val)?;
    let (cur_data, cb) = match &e.ui.get_key(key).unwrap().specific {
        UiSpecific::Table {
            data,
            cell_edit_callback,
            ..
        } => (data.clone(), cell_edit_callback.clone()),
        _ => unreachable!(),
    };
    let (updated_data, prev_cell, new_cell) = update_table_cell(e, cur_data, r, c, edit_val)?;
    if let Some(node) = e.ui.get_mut_key(key)
        && let UiSpecific::Table { data, .. } = &mut node.specific
    {
        *data = updated_data;
    }
    if let Some(cb) = cb {
        let src = handle_val(e, h.clone())?;
        let indices = Value::Matrix(Matrix::from_column_slice(1, 2, &[r as f64, c as f64])?);
        let ev = make_event(
            e,
            src.clone(),
            "CellEdit",
            &[
                ("Indices", indices.clone()),
                ("DisplayIndices", indices),
                ("PreviousData", prev_cell),
                ("EditData", edit_val.clone()),
                ("NewData", new_cell),
                ("Error", Value::Text(String::new())),
            ],
        )?;
        invoke_callback(e, "CellEditCallback", &cb, src, ev, out)?;
    }
    Ok(())
}

pub(super) fn fire_select(
    e: &mut Engine,
    key: usize,
    h: &GraphicsHandle,
    kind: UiKind,
    rest: &[Value],
    out: &mut Execution,
) -> Result<()> {
    if kind != UiKind::Table {
        return Err("mrUiFire 'select' requires a uitable.".into());
    }
    let [idx_val] = rest else {
        return Err("mrUiFire(tbl, 'select', [r c]) requires cell indices.".into());
    };
    let (r, c) = parse_indices(idx_val)?;
    let cb = match &e.ui.get_key(key).unwrap().specific {
        UiSpecific::Table {
            cell_selection_callback,
            ..
        } => cell_selection_callback.clone(),
        _ => None,
    };
    if let Some(cb) = cb {
        let src = handle_val(e, h.clone())?;
        let indices = Value::Matrix(Matrix::from_column_slice(1, 2, &[r as f64, c as f64])?);
        let ev = make_event(e, src.clone(), "CellSelection", &[("Indices", indices)])?;
        invoke_callback(e, "CellSelectionCallback", &cb, src, ev, out)?;
    }
    Ok(())
}

fn update_table_cell(
    e: &Engine,
    data: Value,
    row: usize,
    col: usize,
    new_val: &Value,
) -> Result<(Value, Value, Value)> {
    let r = row - 1;
    let c = col - 1;
    match data {
        Value::Matrix(m) => {
            if r >= m.rows() || c >= m.cols() {
                return Err(format!(
                    "Cell index [{row} {col}] is out of bounds for {}x{} Data.",
                    m.rows(),
                    m.cols()
                ));
            }
            let prev = Value::Matrix(Matrix::scalar(m.get(r, c)));
            let num = match new_val {
                Value::Matrix(nm) if nm.len() == 1 => nm.data()[0],
                Value::Logical(lm) if lm.len() == 1 => f64::from(lm.data()[0]),
                _ => return Err("Editing a numeric table requires a real scalar value.".into()),
            };
            let mut raw = m.data().to_vec();
            raw[c * m.rows() + r] = num;
            let updated = Value::Matrix(Matrix::from_column_vec(m.rows(), m.cols(), raw)?);
            Ok((updated, prev, Value::Matrix(Matrix::scalar(num))))
        }
        Value::Logical(m) => {
            if r >= m.rows() || c >= m.cols() {
                return Err(format!(
                    "Cell index [{row} {col}] is out of bounds for {}x{} Data.",
                    m.rows(),
                    m.cols()
                ));
            }
            let prev = Value::Logical(LogicalMatrix::scalar(m.data()[c * m.rows() + r] != 0));
            let b = match new_val {
                Value::Logical(lm) => lm.as_scalar()?,
                Value::Matrix(nm) if nm.as_scalar().ok() == Some(1.0) => true,
                Value::Matrix(nm) if nm.as_scalar().ok() == Some(0.0) => false,
                _ => return Err("Editing a logical table requires a logical scalar value.".into()),
            };
            let mut raw = m.data().to_vec();
            raw[c * m.rows() + r] = u8::from(b);
            let updated = Value::Logical(LogicalMatrix::from_column_vec(
                m.rows(),
                m.cols(),
                raw,
                &e.cancel,
            )?);
            Ok((updated, prev, Value::Logical(LogicalMatrix::scalar(b))))
        }
        Value::Container(Array::Cell(cell)) => {
            if r >= cell.rows() || c >= cell.cols() {
                return Err(format!(
                    "Cell index [{row} {col}] is out of bounds for {}x{} Data.",
                    cell.rows(),
                    cell.cols()
                ));
            }
            let idx = c * cell.rows() + r;
            let prev = Value::from(cell.get_linear(idx)?.as_ref().clone());
            let new_arr = new_val.clone().array(&e.cancel)?;
            let updated_cell = cell.replaced(idx, new_arr, &e.cancel)?;
            Ok((
                Value::Container(Array::Cell(updated_cell)),
                prev,
                new_val.clone(),
            ))
        }
        Value::Container(Array::Table(tbl)) => {
            let parts = tbl.parts();
            if r >= parts.height || c >= parts.columns.len() {
                return Err(format!(
                    "Cell index [{row} {col}] is out of bounds for {}x{} table Data.",
                    parts.height,
                    parts.columns.len()
                ));
            }
            let mut new_parts = parts.clone();
            let (prev, stored) = match &new_parts.columns[c] {
                Array::Numeric(m) if m.cols() == 1 => {
                    let prev = Value::Matrix(Matrix::scalar(m.get(r, 0)));
                    let num = match new_val {
                        Value::Matrix(nm) if nm.len() == 1 => nm.data()[0],
                        _ => {
                            return Err(
                                "Editing a numeric table column requires a real scalar.".into()
                            );
                        }
                    };
                    let mut raw = m.data().to_vec();
                    raw[r] = num;
                    new_parts.columns[c] = Matrix::from_column_vec(m.rows(), 1, raw)?.into();
                    (prev, Value::Matrix(Matrix::scalar(num)))
                }
                Array::Logical(m) if m.cols() == 1 => {
                    let prev = Value::Logical(LogicalMatrix::scalar(m.data()[r] != 0));
                    let b = match new_val {
                        Value::Logical(lm) => lm.as_scalar()?,
                        Value::Matrix(nm) if nm.as_scalar().ok() == Some(1.0) => true,
                        Value::Matrix(nm) if nm.as_scalar().ok() == Some(0.0) => false,
                        _ => {
                            return Err(
                                "Editing a logical table column requires a logical scalar.".into(),
                            );
                        }
                    };
                    let mut raw = m.data().to_vec();
                    raw[r] = u8::from(b);
                    new_parts.columns[c] =
                        LogicalMatrix::from_column_vec(m.rows(), 1, raw, &e.cancel)?.into();
                    (prev, Value::Logical(LogicalMatrix::scalar(b)))
                }
                Array::Cell(cell) if cell.cols() == 1 => {
                    let prev = Value::from(cell.get_linear(r)?.as_ref().clone());
                    let new_arr = new_val.clone().array(&e.cancel)?;
                    new_parts.columns[c] = cell.replaced(r, new_arr, &e.cancel)?.into();
                    (prev, new_val.clone())
                }
                Array::Str(s) if s.cols() == 1 => {
                    let prev = Value::Text(s.data()[r].as_deref().unwrap_or("").to_string());
                    let text =
                        as_text(new_val).ok_or("Editing a string table column requires text.")?;
                    let mut vals = s.data().to_vec();
                    vals[r] = Some(Arc::from(text.as_str()));
                    new_parts.columns[c] = Array::Str(crate::StringArray::new(s.rows(), 1, vals)?);
                    (prev, Value::Text(text))
                }
                _ => return Err("Editing this table column type is not supported yet.".into()),
            };
            let updated_tbl = TableArray::new(new_parts)?;
            Ok((Value::Container(Array::Table(updated_tbl)), prev, stored))
        }
        _ => Err("Table Data must be a numeric, logical, cell, or table array to edit.".into()),
    }
}
