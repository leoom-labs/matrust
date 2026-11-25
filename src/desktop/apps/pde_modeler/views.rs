//! Figure views, background plotting, the DOF limit, and export-name helpers
//! for PDE Modeler (8c3).
use crate::{figure_geometry::Geometry, figure_view};
use eframe::egui;
use matrust::{Array, Matrix, plot::Figure};
use std::sync::atomic::AtomicBool;

/// Maximum DOFs for time-dependent solves (matches 8b dense ode15s limit).
pub(crate) const TIME_DOF_LIMIT: usize = 4096;

/// Main view tabs in PDE Modeler.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum MainTab {
    #[default]
    Draw,
    Geometry,
    Mesh,
    Solution,
}

impl MainTab {
    pub const ALL: [Self; 4] = [Self::Draw, Self::Geometry, Self::Mesh, Self::Solution];

    pub fn label(self) -> &'static str {
        match self {
            Self::Draw => "Draw",
            Self::Geometry => "Geometry",
            Self::Mesh => "Mesh",
            Self::Solution => "Solution",
        }
    }

    pub fn caption(self) -> &'static str {
        match self {
            Self::Draw => "DRAW",
            Self::Geometry => "GEOMETRY",
            Self::Mesh => "MESH",
            Self::Solution => "SOLUTION",
        }
    }
}

/// Display options for the Solution tab (`pdeplot`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct SolutionPlotOptions {
    pub contour: bool,
    pub mesh: bool,
    pub surface_3d: bool,
}

/// One figure produced by a background engine script together with its prepared
/// display geometry so the UI thread never blocks on geometry preparation.
pub(crate) struct PreparedFigure {
    pub figure: Figure,
    pub geometry: Geometry,
    pub serial: u64,
}

pub(crate) fn prepare_figure(
    figure: Figure,
    cancel: &AtomicBool,
    serial: u64,
) -> Result<PreparedFigure, String> {
    let geometry = Geometry::prepare(1, &figure, cancel)?;
    Ok(PreparedFigure {
        figure,
        geometry,
        serial,
    })
}

/// Draw a prepared figure inside the current UI region, handling 3-D drag rotation.
pub(crate) fn show_prepared_figure(
    ui: &mut egui::Ui,
    view: &mut PreparedFigure,
    plot_id: egui::Id,
) {
    let body = ui.available_rect_before_wrap();
    ui.set_min_size(body.size());
    ui.shrink_clip_rect(body);
    if !ui.clip_rect().is_positive() {
        return;
    }
    let rotate = match &view.geometry {
        Geometry::Ordinary(data) => {
            let shown = figure_view::show_interactive_prepared_fitted_with_id(
                ui,
                plot_id,
                &view.figure,
                data.view(),
                false,
                None,
            );
            shown.rotate
        }
        Geometry::Subplots(_) => None,
    };
    if let Some(delta) = rotate {
        view.figure.axes.view = matrust::plot::view3d::drag_view(view.figure.axes.view, delta);
        let no_cancel = AtomicBool::new(false);
        if let Ok(geometry) = Geometry::prepare(1, &view.figure, &no_cancel) {
            view.geometry = geometry;
        }
    }
}

/// Kind of background job submitted to `host.runner`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunKind {
    Mesh,
    Solve,
}

/// Structured result delivered from a background Mesh or Solve worker task.
pub(crate) struct RunPayload {
    pub kind: RunKind,
    pub geometry_view: Option<PreparedFigure>,
    pub mesh_view: Option<PreparedFigure>,
    pub solution_view: Option<PreparedFigure>,
    pub dl: Option<Matrix>,
    pub mesh_nodes: Option<Array>,
    pub mesh_elems: Option<Array>,
    pub node_count: Option<usize>,
    pub element_count: Option<usize>,
    pub nodal_solution: Option<Array>,
    pub eigenvalues: Option<Vec<f64>>,
    pub tlist: Option<Vec<f64>>,
}

/// Build a `pdeplot(...)` call for either a `model` variable or `(nodes, elems)`.
pub(crate) fn pdeplot_call(mesh_args: &str, u_expr: &str, opts: SolutionPlotOptions) -> String {
    let contour = if opts.contour { "on" } else { "off" };
    let mesh = if opts.mesh { "on" } else { "off" };
    if opts.surface_3d {
        format!(
            "pdeplot({mesh_args}, 'XYData', {u_expr}, 'ZData', {u_expr}, 'ColorBar', 'on', 'Contour', '{contour}', 'Mesh', '{mesh}');\n"
        )
    } else {
        format!(
            "pdeplot({mesh_args}, 'XYData', {u_expr}, 'ColorBar', 'on', 'Contour', '{contour}', 'Mesh', '{mesh}');\n"
        )
    }
}

/// Global `[min, max]` colour limits over every finite entry of `NodalSolution`.
/// Falls back to `[min - 1, min + 1]` only when `max == min`.
pub(crate) fn global_solution_clim(sol: &Array) -> Option<[f64; 2]> {
    let Array::Numeric(m) = sol else {
        return None;
    };
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for &v in m.data() {
        if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return None;
    }
    if hi == lo {
        Some([lo - 1.0, lo + 1.0])
    } else {
        Some([lo, hi])
    }
}

/// Strip a leading `"Line N, column M: "` script-location prefix from an engine error.
fn strip_line_col_prefix(mut line: &str) -> &str {
    loop {
        let Some(rest) = line.strip_prefix("Line ") else {
            return line;
        };
        let digits1 = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits1 == 0 {
            return line;
        }
        let Some(rest) = rest[digits1..].strip_prefix(", column ") else {
            return line;
        };
        let digits2 = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits2 == 0 {
            return line;
        }
        let Some(rest) = rest[digits2..].strip_prefix(':') else {
            return line;
        };
        line = rest.trim_start();
    }
}

/// Extract a single clean line from an engine or validation error message.
pub(crate) fn first_line(msg: &str) -> String {
    let line = msg
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Operation failed.");
    let stripped = strip_line_col_prefix(line);
    if stripped.is_empty() {
        "Operation failed.".to_owned()
    } else {
        stripped.to_owned()
    }
}

/// Replace identifier tokens `model` and `results` outside comments and single-
/// quoted string literals when the user chooses custom export variable names.
pub(crate) fn rename_script_vars(script: &str, model_name: &str, results_name: &str) -> String {
    if model_name == "model" && results_name == "results" {
        return script.to_owned();
    }
    let mut out = String::with_capacity(script.len() + 32);
    for line in script.split_inclusive('\n') {
        if line.trim_start().starts_with('%') {
            out.push_str(line);
            continue;
        }
        out.push_str(&replace_line_idents(line, model_name, results_name));
    }
    out
}

fn replace_line_idents(line: &str, model_name: &str, results_name: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    let mut in_str = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            out.push(b as char);
            if b == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    out.push('\'');
                    i += 2;
                    continue;
                }
                in_str = false;
            }
            i += 1;
            continue;
        }
        if b == b'%' {
            out.push_str(&line[i..]);
            break;
        }
        if b == b'\'' {
            // Distinguish transpose (after identifier / closing bracket) from string literal.
            let prev = bytes[..i]
                .iter()
                .rev()
                .find(|&&c| c != b' ' && c != b'\t')
                .copied();
            let is_transpose = prev.is_some_and(|c| {
                c.is_ascii_alphanumeric() || matches!(c, b'_' | b')' | b']' | b'}' | b'.')
            });
            out.push('\'');
            if !is_transpose {
                in_str = true;
            }
            i += 1;
            continue;
        }
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let ident = &line[start..i];
            if ident == "model" {
                out.push_str(model_name);
            } else if ident == "results" {
                out.push_str(results_name);
            } else {
                out.push_str(ident);
            }
            continue;
        }
        out.push(b as char);
        i += 1;
    }
    out
}
