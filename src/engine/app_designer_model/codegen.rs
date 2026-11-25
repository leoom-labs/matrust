//! MATLAB `.m` classdef code generator for `AppModel`.

use super::{
    AppComponent, AppComponentKind, AppGridTrack, AppModel, AppValue, GridSpanSpec,
    check_single_line_text,
};
use std::fmt::Write as _;

pub const GENERATED_HEADER_MARKER: &str = "% MatRust App Designer generated file (format v1)";
pub const BODY_START_PREFIX: &str = "% [matrust-app-body:start ";
pub const BODY_END_PREFIX: &str = "% [matrust-app-body:end ";
/// Shared tag of both region markers; a body line carrying it in a comment is refused.
pub const BODY_MARKER_TAG: &str = "[matrust-app-body:";

/// Refuse a method body that has a comment line which could be read as a
/// region marker: such a line would end or start a region on reload.
pub fn check_body_markers(body: &str, method_name: &str) -> Result<(), String> {
    let bad = body
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .find(|l| {
            let t = l.trim_start();
            t.starts_with('%') && t.contains(BODY_MARKER_TAG)
        });
    match bad {
        Some(line) => Err(format!(
            "Body of '{method_name}' has a line that looks like a '% [matrust-app-body:...]' region marker ({}); remove or reword it.",
            line.trim()
        )),
        None => Ok(()),
    }
}

/// Quote a single-line Rust string as a MATLAB single-quoted character vector
/// (`'...'`), doubling embedded single quotes and refusing newlines/NUL so text
/// in properties can never break out of a string literal or inject code.
pub fn quote_matlab_char(s: &str) -> Result<String, String> {
    check_single_line_text(s, "String property")?;
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    Ok(out)
}

pub(crate) fn format_number(x: f64) -> Result<String, String> {
    if x.is_nan() {
        return Err("Numeric property cannot be NaN.".into());
    }
    if x == f64::INFINITY {
        return Ok("Inf".into());
    }
    if x == f64::NEG_INFINITY {
        return Ok("-Inf".into());
    }
    if x == 0.0 {
        return Ok("0".into());
    }
    if x.fract() == 0.0 && x.abs() < 1e15 {
        Ok(format!("{}", x as i64))
    } else {
        Ok(format!("{x}"))
    }
}

fn format_cellstr(items: &[String], sep: &str) -> Result<String, String> {
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        parts.push(quote_matlab_char(item)?);
    }
    Ok(format!("{{{}}}", parts.join(sep)))
}

fn format_grid_tracks(tracks: &[AppGridTrack]) -> Result<String, String> {
    let mut parts = Vec::with_capacity(tracks.len());
    for t in tracks {
        match t {
            AppGridTrack::Pixels(px) => {
                if !px.is_finite() || *px < 0.0 {
                    return Err("Grid track pixel size must be finite and non-negative.".into());
                }
                parts.push(format_number(*px)?);
            }
            AppGridTrack::Weight(w) => {
                if !w.is_finite() || *w <= 0.0 {
                    return Err("Grid track weight must be finite and positive.".into());
                }
                let n = format_number(*w)?;
                parts.push(format!("'{n}x'"));
            }
            AppGridTrack::Fit => parts.push("'fit'".into()),
        }
    }
    Ok(format!("{{{}}}", parts.join(", ")))
}

fn format_span(span: GridSpanSpec) -> String {
    match span {
        GridSpanSpec::Single(n) => n.to_string(),
        GridSpanSpec::Range(a, b) => format!("[{a} {b}]"),
    }
}

fn format_position(pos: [f64; 4]) -> Result<String, String> {
    Ok(format!(
        "[{} {} {} {}]",
        format_number(pos[0])?,
        format_number(pos[1])?,
        format_number(pos[2])?,
        format_number(pos[3])?,
    ))
}

fn constructor_call(kind: AppComponentKind, parent_expr: &str) -> String {
    match kind {
        AppComponentKind::Button => format!("uibutton({parent_expr}, 'push')"),
        AppComponentKind::StateButton => format!("uibutton({parent_expr}, 'state')"),
        AppComponentKind::Label => format!("uilabel({parent_expr})"),
        AppComponentKind::EditField => format!("uieditfield({parent_expr}, 'text')"),
        AppComponentKind::NumericEditField => format!("uieditfield({parent_expr}, 'numeric')"),
        AppComponentKind::TextArea => format!("uitextarea({parent_expr})"),
        AppComponentKind::DropDown => format!("uidropdown({parent_expr})"),
        AppComponentKind::ListBox => format!("uilistbox({parent_expr})"),
        AppComponentKind::CheckBox => format!("uicheckbox({parent_expr})"),
        AppComponentKind::RadioButton => format!("uiradiobutton({parent_expr})"),
        AppComponentKind::Slider => format!("uislider({parent_expr})"),
        AppComponentKind::Spinner => format!("uispinner({parent_expr})"),
        AppComponentKind::Switch => format!("uiswitch({parent_expr})"),
        AppComponentKind::Table => format!("uitable({parent_expr})"),
        AppComponentKind::Axes => format!("uiaxes({parent_expr})"),
        AppComponentKind::Panel => format!("uipanel({parent_expr})"),
        AppComponentKind::TabGroup => format!("uitabgroup({parent_expr})"),
        AppComponentKind::Tab => format!("uitab({parent_expr})"),
        AppComponentKind::ButtonGroup => format!("uibuttongroup({parent_expr})"),
    }
}

fn write_body_block(out: &mut String, name: &str, body: &str) {
    let _ = writeln!(out, "            {BODY_START_PREFIX}{name}]");
    if !body.is_empty() {
        for line in body.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.is_empty() {
                out.push('\n');
            } else {
                let _ = writeln!(out, "            {line}");
            }
        }
    }
    let _ = writeln!(out, "            {BODY_END_PREFIX}{name}]");
}

fn emit_component_props(
    out: &mut String,
    model: &AppModel,
    comp: &AppComponent,
) -> Result<(), String> {
    let name = &comp.name;

    if model.is_in_root_grid(comp) {
        if let Some(row) = comp.grid_row {
            let _ = writeln!(
                out,
                "            app.{name}.Layout.Row = {};",
                format_span(row)
            );
        }
        if let Some(col) = comp.grid_column {
            let _ = writeln!(
                out,
                "            app.{name}.Layout.Column = {};",
                format_span(col)
            );
        }
    } else {
        let _ = writeln!(
            out,
            "            app.{name}.Position = {};",
            format_position(comp.position)?
        );
    }

    // Emit Items / Limits before Value so runtime value validation sees the
    // updated domain first.
    if let Some(ref items) = comp.props.items {
        let _ = writeln!(
            out,
            "            app.{name}.Items = {};",
            format_cellstr(items, ", ")?
        );
    }
    if let Some(lim) = comp.props.limits {
        let _ = writeln!(
            out,
            "            app.{name}.Limits = [{} {}];",
            format_number(lim[0])?,
            format_number(lim[1])?
        );
    }
    if let Some(ref text) = comp.props.text {
        let _ = writeln!(
            out,
            "            app.{name}.Text = {};",
            quote_matlab_char(text)?
        );
    }
    if let Some(ref title) = comp.props.title {
        let _ = writeln!(
            out,
            "            app.{name}.Title = {};",
            quote_matlab_char(title)?
        );
    }
    if let Some(ref cols) = comp.props.column_name {
        let _ = writeln!(
            out,
            "            app.{name}.ColumnName = {};",
            format_cellstr(cols, ", ")?
        );
    }
    if let Some(ref val) = comp.props.value {
        match val {
            AppValue::Number(x) => {
                let _ = writeln!(
                    out,
                    "            app.{name}.Value = {};",
                    format_number(*x)?
                );
            }
            AppValue::Text(s) => {
                let _ = writeln!(
                    out,
                    "            app.{name}.Value = {};",
                    quote_matlab_char(s)?
                );
            }
            AppValue::Bool(b) => {
                let _ = writeln!(
                    out,
                    "            app.{name}.Value = {};",
                    if *b { "true" } else { "false" }
                );
            }
            AppValue::Lines(lines) => {
                let _ = writeln!(
                    out,
                    "            app.{name}.Value = {};",
                    format_cellstr(lines, "; ")?
                );
            }
        }
    }

    for cb in &model.callbacks {
        if cb.component == comp.name {
            let _ = writeln!(
                out,
                "            app.{name}.{} = createCallbackFcn(app, @{}, true);",
                cb.event, cb.method_name
            );
        }
    }
    Ok(())
}

/// Generate a MATLAB `< matlab.apps.AppBase` classdef file from `model`.
/// Refuses to generate if `model.validate()` reports any problems.
pub fn generate_m(model: &AppModel) -> Result<String, String> {
    let problems = model.validate();
    if !problems.is_empty() {
        let joined = problems
            .iter()
            .map(|p| p.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!("Cannot generate app code: {joined}"));
    }

    let mut out = String::new();
    let _ = writeln!(out, "classdef {} < matlab.apps.AppBase", model.class_name);
    let _ = writeln!(out, "    {GENERATED_HEADER_MARKER}");
    out.push('\n');

    // Public component properties
    let _ = writeln!(out, "    % Properties that correspond to app components");
    let _ = writeln!(out, "    properties (Access = public)");
    let _ = writeln!(out, "        {} matlab.ui.Figure", model.figure.name);
    if let Some(ref grid) = model.grid {
        let _ = writeln!(out, "        {} matlab.ui.container.GridLayout", grid.name);
    }
    for comp in &model.components {
        let _ = writeln!(
            out,
            "        {} {}",
            comp.name,
            comp.kind.matlab_class_name()
        );
    }
    let _ = writeln!(out, "    end");

    // Private properties
    if !model.private_properties.is_empty() {
        out.push('\n');
        let _ = writeln!(out, "    properties (Access = private)");
        for prop in &model.private_properties {
            if let Some(ref expr) = prop.default_expr {
                let _ = writeln!(out, "        {} = {}", prop.name, expr);
            } else {
                let _ = writeln!(out, "        {}", prop.name);
            }
        }
        let _ = writeln!(out, "    end");
    }

    // Callbacks and startupFcn
    if model.startup_fcn.is_some() || !model.callbacks.is_empty() {
        out.push('\n');
        let _ = writeln!(out, "    % Callbacks that handle component events");
        let _ = writeln!(out, "    methods (Access = private)");
        if let Some(ref body) = model.startup_fcn {
            out.push('\n');
            let _ = writeln!(out, "        % Code that executes after component creation");
            let _ = writeln!(out, "        function startupFcn(app)");
            write_body_block(&mut out, "startupFcn", body);
            let _ = writeln!(out, "        end");
        }
        for cb in &model.callbacks {
            out.push('\n');
            let _ = writeln!(out, "        % Callback: {}.{}", cb.component, cb.event);
            let _ = writeln!(out, "        function {}(app, event)", cb.method_name);
            write_body_block(&mut out, &cb.method_name, &cb.body);
            let _ = writeln!(out, "        end");
        }
        let _ = writeln!(out, "    end");
    }

    // Component initialization
    out.push('\n');
    let _ = writeln!(out, "    % Component initialization");
    let _ = writeln!(out, "    methods (Access = private)");
    out.push('\n');
    let _ = writeln!(out, "        % Create UIFigure and components");
    let _ = writeln!(out, "        function createComponents(app)");
    let fig_name = &model.figure.name;
    let _ = writeln!(
        out,
        "            app.{fig_name} = uifigure('Visible', 'off');"
    );
    let _ = writeln!(
        out,
        "            app.{fig_name}.Position = {};",
        format_position(model.figure.position)?
    );
    let _ = writeln!(
        out,
        "            app.{fig_name}.Name = {};",
        quote_matlab_char(&model.figure.title)?
    );
    for cb in &model.callbacks {
        if cb.component == model.figure.name {
            let _ = writeln!(
                out,
                "            app.{fig_name}.{} = createCallbackFcn(app, @{}, true);",
                cb.event, cb.method_name
            );
        }
    }

    if let Some(ref grid) = model.grid {
        out.push('\n');
        let gname = &grid.name;
        let _ = writeln!(
            out,
            "            app.{gname} = uigridlayout(app.{fig_name});"
        );
        let _ = writeln!(
            out,
            "            app.{gname}.RowHeight = {};",
            format_grid_tracks(&grid.row_height)?
        );
        let _ = writeln!(
            out,
            "            app.{gname}.ColumnWidth = {};",
            format_grid_tracks(&grid.column_width)?
        );
    }

    for comp in &model.components {
        out.push('\n');
        let parent_expr = match comp.parent.as_deref() {
            Some(p) => format!("app.{p}"),
            None => match model.grid {
                Some(ref g) => format!("app.{}", g.name),
                None => format!("app.{fig_name}"),
            },
        };
        let _ = writeln!(
            out,
            "            app.{} = {};",
            comp.name,
            constructor_call(comp.kind, &parent_expr)
        );
        emit_component_props(&mut out, model, comp)?;
    }

    out.push('\n');
    let _ = writeln!(out, "            app.{fig_name}.Visible = 'on';");
    let _ = writeln!(out, "        end");
    let _ = writeln!(out, "    end");

    // App creation and deletion
    out.push('\n');
    let _ = writeln!(out, "    % App creation and deletion");
    let _ = writeln!(out, "    methods (Access = public)");
    out.push('\n');
    let _ = writeln!(out, "        % Construct app");
    let _ = writeln!(out, "        function app = {}", model.class_name);
    let _ = writeln!(out, "            createComponents(app)");
    let _ = writeln!(out, "            registerApp(app, app.{fig_name})");
    if model.startup_fcn.is_some() {
        let _ = writeln!(out, "            runStartupFcn(app, @startupFcn)");
    }
    out.push('\n');
    let _ = writeln!(out, "            if nargout == 0");
    let _ = writeln!(out, "                clear app");
    let _ = writeln!(out, "            end");
    let _ = writeln!(out, "        end");
    out.push('\n');
    let _ = writeln!(out, "        % Code that executes before app deletion");
    let _ = writeln!(out, "        function delete(app)");
    let _ = writeln!(out, "            delete(app.{fig_name})");
    let _ = writeln!(out, "        end");
    let _ = writeln!(out, "    end");
    let _ = writeln!(out, "end");

    Ok(out)
}
