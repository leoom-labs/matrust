//! UI dialog builtins routed through `src/engine/dialogs.rs` host requests:
//! `uialert`, `uiconfirm`, and `uiprogressdlg`.
use super::callbacks::{invoke_callback, make_event, parse_callback};
use super::props::{parse_cellstr, parse_on_off, parse_real_scalar, parse_text_bounded};
use super::{UiCallback, UiKind, UiNode, UiSpecific, as_text};
use crate::engine::dialogs::{DialogReply, DialogRequest, MessageIcon};
use crate::engine::{Engine, Execution, Value};
use crate::{Array, GraphicsHandle, GraphicsKind, Matrix, ObjectIdentity, Result};

enum DialogParent {
    UiFigure(usize, GraphicsHandle),
    ClassicFigure(ObjectIdentity, GraphicsHandle),
}

fn require_figure_parent(e: &Engine, arg: &Value, fn_name: &str) -> Result<DialogParent> {
    let Value::Container(Array::Graphics(arr)) = arg else {
        return Err(format!(
            "{fn_name} requires a figure handle as its first argument."
        ));
    };
    let h = arr.scalar_handle()?.clone();
    match h.kind() {
        GraphicsKind::UiComponent => {
            let key = h.object.recovery_key();
            let Some(node) = e.ui.get_key(key) else {
                return Err("Invalid or deleted object.".into());
            };
            if node.kind != UiKind::Figure {
                return Err(format!(
                    "{fn_name} requires a figure handle as its first argument."
                ));
            }
            Ok(DialogParent::UiFigure(key, h))
        }
        GraphicsKind::Figure => {
            let located = e.locate_graphics(&h)?;
            let fig = e.graphics_figure(located.figure)?;
            Ok(DialogParent::ClassicFigure(fig.identity.clone(), h))
        }
        _ => Err(format!(
            "{fn_name} requires a figure handle as its first argument."
        )),
    }
}

fn parse_message_lines(e: &Engine, v: &Value, prop: &str) -> Result<Vec<String>> {
    if let Some(s) = as_text(v) {
        return Ok(s.split('\n').map(str::to_owned).collect());
    }
    let list = parse_cellstr(e, v, prop)?;
    Ok(list
        .iter()
        .flat_map(|s| s.split('\n').map(str::to_owned).collect::<Vec<_>>())
        .collect())
}

fn parse_icon(v: &Value) -> Result<(String, MessageIcon)> {
    let raw = parse_text_bounded(v, "Icon")?;
    let lower = raw.to_ascii_lowercase();
    let icon = match lower.as_str() {
        "error" => MessageIcon::Error,
        "warning" | "warn" => MessageIcon::Warn,
        "info" | "question" | "help" => MessageIcon::Help,
        "none" | "success" | "" => MessageIcon::None,
        _ => {
            return Err(format!(
                "Icon must be 'error', 'warning', 'info', 'success', 'question', or 'none', got '{raw}'."
            ));
        }
    };
    Ok((lower, icon))
}

fn parse_interpreter(v: &Value) -> Result<String> {
    let raw = parse_text_bounded(v, "Interpreter")?;
    let lower = raw.to_ascii_lowercase();
    if !matches!(lower.as_str(), "none" | "tex" | "latex") {
        return Err("Interpreter must be 'none', 'tex', or 'latex'.".into());
    }
    Ok(lower)
}

fn resolve_option_index(v: &Value, options: &[String], prop: &str) -> Result<usize> {
    if let Some(s) = as_text(v) {
        if let Some(pos) = options.iter().position(|opt| opt == &s) {
            return Ok(pos + 1);
        }
        return Err(format!("{prop} '{s}' does not match any entry in Options."));
    }
    let n = parse_real_scalar(v, prop)?;
    if !n.is_finite() || n.fract() != 0.0 || n < 1.0 || n as usize > options.len() {
        return Err(format!(
            "{prop} must be an integer between 1 and {} or an option string.",
            options.len()
        ));
    }
    Ok(n as usize)
}

/// `uialert(fig, message, title, Name, Value, ...)`
pub(super) fn call_uialert(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    if args.len() < 3 {
        return Err(
            "uialert requires at least three arguments: uialert(fig, message, title).".into(),
        );
    }
    let parent = require_figure_parent(e, &args[0], "uialert")?;
    let lines = parse_message_lines(e, &args[1], "Message")?;
    let title = parse_text_bounded(&args[2], "Title")?;
    let nv = &args[3..];
    if !nv.len().is_multiple_of(2) {
        return Err("uialert expects Name, Value property pairs.".into());
    }
    let mut icon = MessageIcon::Error;
    let mut close_fcn: Option<UiCallback> = None;
    for pair in nv.as_chunks::<2>().0 {
        let name = as_text(&pair[0]).ok_or("Property name must be character text.")?;
        match name.to_ascii_lowercase().as_str() {
            "icon" => icon = parse_icon(&pair[1])?.1,
            "closefcn" => close_fcn = parse_callback(e, &pair[1])?,
            "interpreter" => {
                let _ = parse_interpreter(&pair[1])?;
            }
            "modal" => {
                let _ = parse_on_off(&pair[1], "Modal")?;
            }
            _ => return Err(format!("uialert: unsupported property '{name}'.")),
        }
    }
    let parent_fig = match &parent {
        DialogParent::UiFigure(k, _) => *k,
        DialogParent::ClassicFigure(id, _) => id.recovery_key(),
    };
    e.ui.next_alert_id = e.ui.next_alert_id.wrapping_add(1);
    let id = e.ui.next_alert_id;
    e.ui.pending_alert_figs.insert(id, parent_fig);
    e.gobj_flush_frame();
    let request = DialogRequest::UiAlert {
        id,
        parent_fig,
        title,
        text: lines,
        icon,
    };
    let reply = match e.dlg_ask_raw(request, out) {
        Ok(r) => r,
        Err(err) => {
            e.ui.pending_alert_figs.remove(&id);
            return Err(err);
        }
    };
    match reply {
        DialogReply::Failed(why) => {
            e.ui.pending_alert_figs.remove(&id);
            return Err(format!("uialert: {why}"));
        }
        DialogReply::Unsupported => {
            e.ui.pending_alert_figs.remove(&id);
            // In -batch / headless, run CloseFcn immediately and return.
            if let Some(cb) = close_fcn {
                let fig_handle = match parent {
                    DialogParent::UiFigure(_, h) | DialogParent::ClassicFigure(_, h) => h,
                };
                let src = super::super::graphics_handles::dot_syntax::handle_value(
                    fig_handle, &e.cancel,
                )?;
                let ev = make_event(e, src.clone(), "Close", &[])?;
                invoke_callback(e, "CloseFcn", &cb, src, ev, out)?;
            }
        }
        _ => {
            // Non-blocking desktop sheet: defer CloseFcn until the user dismisses OK.
            if let Some(cb) = close_fcn {
                let fig_handle = match parent {
                    DialogParent::UiFigure(_, h) | DialogParent::ClassicFigure(_, h) => h,
                };
                e.ui.pending_alert_close.insert(id, (fig_handle, cb));
            }
        }
    }
    Ok(Value::Void)
}

/// `selection = uiconfirm(fig, message, title, Name, Value, ...)`
pub(super) fn call_uiconfirm(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    if args.len() < 3 {
        return Err(
            "uiconfirm requires at least three arguments: uiconfirm(fig, message, title).".into(),
        );
    }
    let parent = require_figure_parent(e, &args[0], "uiconfirm")?;
    let lines = parse_message_lines(e, &args[1], "Message")?;
    let title = parse_text_bounded(&args[2], "Title")?;
    let nv = &args[3..];
    if !nv.len().is_multiple_of(2) {
        return Err("uiconfirm expects Name, Value property pairs.".into());
    }
    let mut options = vec!["OK".to_string(), "Cancel".to_string()];
    let mut default_raw: Option<Value> = None;
    let mut cancel_raw: Option<Value> = None;
    let mut icon = MessageIcon::None;
    let mut close_fcn: Option<UiCallback> = None;
    for pair in nv.as_chunks::<2>().0 {
        let name = as_text(&pair[0]).ok_or("Property name must be character text.")?;
        match name.to_ascii_lowercase().as_str() {
            "options" => {
                let parsed = parse_cellstr(e, &pair[1], "Options")?;
                if parsed.is_empty() {
                    return Err("uiconfirm Options must not be empty.".into());
                }
                options = parsed;
            }
            "defaultoption" => default_raw = Some(pair[1].clone()),
            "canceloption" => cancel_raw = Some(pair[1].clone()),
            "icon" => {
                icon = parse_icon(&pair[1])?.1;
            }
            "closefcn" => close_fcn = parse_callback(e, &pair[1])?,
            "interpreter" => {
                let _ = parse_interpreter(&pair[1])?;
            }
            _ => return Err(format!("uiconfirm: unsupported property '{name}'.")),
        }
    }
    let default_idx = match default_raw {
        Some(v) => resolve_option_index(&v, &options, "DefaultOption")?,
        None => 1,
    };
    let cancel_idx = match cancel_raw {
        Some(v) => Some(resolve_option_index(&v, &options, "CancelOption")?),
        None => options
            .iter()
            .position(|o| o.eq_ignore_ascii_case("cancel"))
            .map(|p| p + 1),
    };
    let default_text = options[default_idx - 1].clone();
    let parent_fig = match &parent {
        DialogParent::UiFigure(k, _) => *k,
        DialogParent::ClassicFigure(id, _) => id.recovery_key(),
    };
    *e.ui.active_confirm_figs.entry(parent_fig).or_insert(0) += 1;
    e.gobj_flush_frame();
    let request = DialogRequest::UiConfirm {
        parent_fig,
        title,
        text: lines,
        options: options.clone(),
        default: default_text.clone(),
        icon,
    };
    let ask_result = e.dlg_ask_raw(request, out);
    if let Some(cnt) = e.ui.active_confirm_figs.get_mut(&parent_fig) {
        *cnt = cnt.saturating_sub(1);
        if *cnt == 0 {
            e.ui.active_confirm_figs.remove(&parent_fig);
        }
    }
    let (selected_text, selected_idx) = match ask_result? {
        DialogReply::Button(chosen) => {
            let idx = options
                .iter()
                .position(|o| o == &chosen)
                .map_or(0, |p| p + 1);
            (chosen, idx)
        }
        DialogReply::Cancelled => match cancel_idx {
            Some(ci) => (options[ci - 1].clone(), ci),
            None => (String::new(), 0),
        },
        DialogReply::Unsupported => (default_text, default_idx),
        DialogReply::Failed(why) => return Err(format!("uiconfirm: {why}")),
        other => return Err(format!("uiconfirm: unexpected host reply {other:?}")),
    };
    if let Some(cb) = close_fcn {
        let fig_handle = match parent {
            DialogParent::UiFigure(_, h) | DialogParent::ClassicFigure(_, h) => h,
        };
        let src = super::super::graphics_handles::dot_syntax::handle_value(fig_handle, &e.cancel)?;
        let ev = make_event(
            e,
            src.clone(),
            "ConfirmClose",
            &[
                ("SelectedOption", Value::Text(selected_text.clone())),
                (
                    "SelectedOptionIndex",
                    Value::Matrix(Matrix::scalar(selected_idx as f64)),
                ),
            ],
        )?;
        invoke_callback(e, "CloseFcn", &cb, src, ev, out)?;
    }
    Ok(Value::Text(selected_text))
}

/// `d = uiprogressdlg(fig, Name, Value, ...)`
pub(super) fn call_uiprogressdlg(
    e: &mut Engine,
    args: &[Value],
    out: &mut Execution,
) -> Result<Value> {
    if args.is_empty() {
        return Err("uiprogressdlg requires a figure handle as its first argument.".into());
    }
    let parent = require_figure_parent(e, &args[0], "uiprogressdlg")?;
    let nv = &args[1..];
    if !nv.len().is_multiple_of(2) {
        return Err("uiprogressdlg expects Name, Value property pairs.".into());
    }
    let mut title = String::new();
    let mut message = String::new();
    let mut value = 0.0_f64;
    let mut indeterminate = false;
    let mut cancelable = false;
    let mut cancel_text = "Cancel".to_string();
    let mut show_percentage = false;
    let mut icon = "none".to_string();
    let mut interpreter = "none".to_string();
    for pair in nv.as_chunks::<2>().0 {
        let name = as_text(&pair[0]).ok_or("Property name must be character text.")?;
        match name.to_ascii_lowercase().as_str() {
            "title" => title = parse_text_bounded(&pair[1], "Title")?,
            "message" => {
                let lines = parse_message_lines(e, &pair[1], "Message")?;
                message = lines.join("\n");
            }
            "value" => {
                let v = parse_real_scalar(&pair[1], "Value")?;
                if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                    return Err("uiprogressdlg Value must be a scalar in [0, 1].".into());
                }
                value = v;
            }
            "indeterminate" => indeterminate = parse_on_off(&pair[1], "Indeterminate")?,
            "cancelable" => cancelable = parse_on_off(&pair[1], "Cancelable")?,
            "canceltext" => cancel_text = parse_text_bounded(&pair[1], "CancelText")?,
            "showpercentage" => show_percentage = parse_on_off(&pair[1], "ShowPercentage")?,
            "icon" => icon = parse_icon(&pair[1])?.0,
            "interpreter" => interpreter = parse_interpreter(&pair[1])?,
            _ => return Err(format!("uiprogressdlg: unsupported property '{name}'.")),
        }
    }
    let dlg_id = e.dlg_new_handle()?;
    let headless = e
        .host_io
        .as_ref()
        .and_then(|host| host.lock().ok().map(|h| !h.supports_ui_wait()))
        .unwrap_or(true);
    let _ = out;
    let specific = UiSpecific::ProgressDialog {
        dlg_id,
        title,
        message,
        value,
        indeterminate,
        cancelable,
        cancel_text,
        cancel_requested: false,
        show_percentage,
        icon,
        interpreter,
        headless,
    };
    let node = match parent {
        DialogParent::UiFigure(pk, _) => UiNode::new(UiKind::ProgressDialog, Some(pk), specific),
        DialogParent::ClassicFigure(fig_id, _) => {
            let mut n = UiNode::new(UiKind::ProgressDialog, None, specific);
            n.classic_parent = Some(fig_id);
            n
        }
    };
    let classic_fig_key = node.classic_parent.as_ref().map(|id| id.recovery_key());
    let key = e.ui.insert_node(node)?;
    if let Some(cfk) = classic_fig_key {
        e.ui.classic_fig_children.entry(cfk).or_default().push(key);
    }
    if !headless {
        e.gobj_flush_frame();
    }
    let handle = e.ui.get_key(key).unwrap().handle();
    super::super::graphics_handles::dot_syntax::handle_value(handle, &e.cancel)
}

/// Flush the UI snapshot to the host when a non-headless progress dialog's
/// Title, Message, or Value changes.
pub(super) fn sync_progress_host(
    e: &mut Engine,
    _dlg_id: u32,
    _title: String,
    _message: &str,
    _value: f64,
    headless: bool,
) -> Result<()> {
    if !headless {
        e.gobj_flush_frame();
    }
    Ok(())
}

/// Flush the UI snapshot to the host when closing a non-headless progress dialog.
pub(super) fn close_progress_host(
    e: &mut Engine,
    _dlg_id: u32,
    headless: bool,
    _out: &mut Execution,
) -> Result<()> {
    if !headless {
        e.gobj_flush_frame();
    }
    Ok(())
}
