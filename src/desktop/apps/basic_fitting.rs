//! Basic Fitting: a companion for existing figure lines. All fitting,
//! evaluation and graphics are engine scripts; the UI only formats results.
use super::{App, AppKind, Host, Outputs, Snapshot, field, number, unused_name};
use crate::toolkits::Action;
use eframe::egui;
use matrust::plot::{Figure, PlotKind, Position, SeriesPoints};
use std::collections::BTreeMap;

pub const MAX_DATA: usize = 20_000;
const MAX_EVALUATE: usize = 2_000;
const PREVIEW_POINTS: usize = 401;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitKind {
    Spline,
    Pchip,
    Polynomial(usize),
}
impl FitKind {
    pub const ALL: [Self; 11] = [
        Self::Spline,
        Self::Pchip,
        Self::Polynomial(1),
        Self::Polynomial(2),
        Self::Polynomial(3),
        Self::Polynomial(4),
        Self::Polynomial(5),
        Self::Polynomial(6),
        Self::Polynomial(7),
        Self::Polynomial(8),
        Self::Polynomial(9),
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Spline => "spline",
            Self::Pchip => "pchip",
            Self::Polynomial(1) => "linear",
            Self::Polynomial(2) => "quadratic",
            Self::Polynomial(3) => "cubic",
            Self::Polynomial(4) => "4th degree",
            Self::Polynomial(5) => "5th degree",
            Self::Polynomial(6) => "6th degree",
            Self::Polynomial(7) => "7th degree",
            Self::Polynomial(8) => "8th degree",
            Self::Polynomial(_) => "9th degree",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResidualStyle {
    Bar,
    Scatter,
    Line,
}
impl ResidualStyle {
    fn command(self) -> &'static str {
        match self {
            Self::Bar => "bar",
            Self::Scatter => "scatter",
            Self::Line => "plot",
        }
    }
}
#[derive(Clone)]
pub struct LineChoice {
    pub figure: u32,
    pub axes: Position,
    pub axis_number: usize,
    /// One-based index within findobj(figure, 'Type', 'line').
    pub ordinal: usize,
    pub label: String,
    is_3d: bool,
    identity: matrust::ObjectIdentity,
    points: SeriesPoints,
}
impl LineChoice {
    pub fn caption(&self) -> String {
        format!(
            "Figure {} · axes {} · {}",
            self.figure, self.axis_number, self.label
        )
    }
}
#[derive(Clone, Debug)]
pub struct FitResult {
    pub kind: FitKind,
    pub coefficients: Vec<f64>,
    pub mu: Vec<f64>,
    pub norm: f64,
    pub breaks: usize,
    pub coefficient_count: usize,
    pub curve: Vec<[f64; 2]>,
    pub residuals: Vec<[f64; 2]>,
    pub evaluated: Vec<[f64; 2]>,
    pub formula: String,
}
#[derive(Clone, Debug)]
pub struct ResultSet {
    pub data: Vec<[f64; 2]>,
    pub fits: Vec<FitResult>,
}
pub struct BasicFitting {
    pub workspace_data: bool,
    pub x: String,
    pub y: String,
    pub lines: Vec<LineChoice>,
    pub selected_line: usize,
    pub enabled: [bool; 11],
    pub normalize: bool,
    pub digits: usize,
    pub equations: bool,
    pub residuals: bool,
    pub residual_style: ResidualStyle,
    pub separate_residuals: bool,
    pub show_norm: bool,
    pub range_min: String,
    pub range_max: String,
    pub evaluate: String,
    pub evaluate_fit: usize,
    pub plot_evaluated: bool,
    /// Blank names are allocated from fit, fit1, ... without replacement.
    pub fit_names: [String; 11],
    pub evaluated_x_name: String,
    pub evaluated_y_name: String,
    pub current: Option<ResultSet>,
    apply_requested: bool,
    can_apply: bool,
    eval_inputs: Vec<String>,
    seed: String,
    seeded: Option<(u32, usize, SeriesPoints)>,
}
impl Default for BasicFitting {
    fn default() -> Self {
        let mut enabled = [false; 11];
        enabled[3] = true;
        Self {
            workspace_data: false,
            x: String::new(),
            y: String::new(),
            lines: Vec::new(),
            selected_line: 0,
            enabled,
            normalize: false,
            digits: 5,
            equations: false,
            residuals: true,
            residual_style: ResidualStyle::Scatter,
            separate_residuals: false,
            show_norm: true,
            range_min: String::new(),
            range_max: String::new(),
            evaluate: String::new(),
            evaluate_fit: 3,
            plot_evaluated: false,
            fit_names: Default::default(),
            evaluated_x_name: String::new(),
            evaluated_y_name: String::new(),
            current: None,
            apply_requested: false,
            can_apply: false,
            eval_inputs: Vec::new(),
            seed: String::new(),
            seeded: None,
        }
    }
}
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}
fn literal(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v == f64::INFINITY {
        "Inf".into()
    } else if v == f64::NEG_INFINITY {
        "-Inf".into()
    } else {
        number(v)
    }
}
fn vector(values: impl Iterator<Item = f64>) -> String {
    format!("[{}]", values.map(literal).collect::<Vec<_>>().join(" "))
}
fn fail(message: &str) -> String {
    format!("Basic Fitting: {message}")
}
fn real_values(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(matrust::Array::Numeric(m)) => Ok(m.data().to_vec()),
        _ => Err(fail(&format!("{name} is not a real array"))),
    }
}
fn pairs(x: &[f64], y: &[f64]) -> Vec<[f64; 2]> {
    x.iter()
        .zip(y)
        .map(|(x, y)| [*x, *y])
        .filter(|p| p.iter().all(|v| v.is_finite()))
        .collect()
}
pub fn significant(value: f64, digits: usize) -> String {
    if !value.is_finite() || value == 0. {
        return super::short(value);
    }
    let digits = digits.clamp(2, 8);
    let magnitude = value.abs().log10().floor() as i32;
    if magnitude < -3 || magnitude >= digits as i32 {
        format!("{value:.precision$e}", precision = digits - 1)
    } else {
        format!(
            "{value:.precision$}",
            precision = (digits as i32 - 1 - magnitude).max(0) as usize
        )
    }
}
/// Display-only equation; numerical evaluation always uses polyval / ppval.
fn formula(kind: FitKind, coeff: &[f64], mu: &[f64], digits: usize) -> String {
    if !matches!(kind, FitKind::Polynomial(_)) {
        return format!("{} piecewise cubic", kind.label());
    }
    let variable = if mu.is_empty() { "x" } else { "z" };
    let terms = coeff
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let power = coeff.len() - 1 - i;
            let c = significant(*c, digits);
            match power {
                0 => c,
                1 => format!("({c})*{variable}"),
                _ => format!("({c})*{variable}^{power}"),
            }
        })
        .collect::<Vec<_>>()
        .join(" + ");
    if mu.len() == 2 {
        format!(
            "y = {terms}; z = (x - {})/{}",
            significant(mu[0], digits),
            significant(mu[1], digits)
        )
    } else {
        format!("y = {terms}")
    }
}
fn expression(text: &str) -> Result<(), String> {
    if text.len() > 256 {
        return Err(fail("evaluation expression is limited to 256 bytes"));
    }
    if text.is_empty() {
        return Ok(());
    }
    super::single_expression(text).map_err(|e| fail(&format!("evaluation: {e}")))
}
impl BasicFitting {
    /// Refresh labels and shared samples, retaining selection by object identity.
    pub fn sync_figures(&mut self, figures: &BTreeMap<u32, Figure>) {
        let old = self
            .lines
            .get(self.selected_line)
            .map(|l| l.identity.clone());
        let mut lines = Vec::new();
        for (&figure, f) in figures {
            let mut axes = f.all_axes().collect::<Vec<_>>();
            axes.sort_by_key(|(p, _)| *p);
            let mut ordinal = 0;
            for (axis, (position, a)) in axes.into_iter().enumerate() {
                for s in &a.series {
                    if s.kind != PlotKind::Line {
                        continue;
                    }
                    ordinal += 1;
                    lines.push(LineChoice {
                        figure,
                        axes: position,
                        axis_number: axis + 1,
                        ordinal,
                        label: if s.label.is_empty() {
                            format!("data {ordinal}")
                        } else {
                            s.label.clone()
                        },
                        is_3d: s.is_3d(),
                        identity: s.identity.clone(),
                        points: s.points.clone(),
                    });
                }
            }
        }
        self.selected_line = old
            .and_then(|id| lines.iter().position(|l| l.identity.same_object(&id)))
            .unwrap_or(0);
        // Metadata can change the source's ordinal or axes, so rebuild the small
        // private scene on metadata changes as well as sample-storage changes.
        let changed = self.lines.len() != lines.len()
            || self.lines.iter().zip(&lines).any(|(a, b)| {
                a.figure != b.figure
                    || a.axes != b.axes
                    || a.ordinal != b.ordinal
                    || a.label != b.label
                    || a.is_3d != b.is_3d
            });
        if changed {
            self.seeded = None;
        }
        self.lines = lines;
    }
    fn selection(&self) -> usize {
        if self
            .enabled
            .get(self.evaluate_fit)
            .copied()
            .unwrap_or(false)
        {
            self.evaluate_fit
        } else {
            self.enabled.iter().position(|v| *v).unwrap_or(0)
        }
    }
    fn prefix(&self, ws: &Snapshot) -> String {
        unused_name("bf", |p| {
            ws.variables()
                .any(|(name, _)| name.starts_with(&format!("{p}_")))
        }) + "_"
    }
    pub fn output_names(&self, ws: &Snapshot) -> Result<Vec<String>, String> {
        let mut names = Vec::new();
        let mut allocate = |edited: &str, base: &str| -> Result<(), String> {
            let name = if edited.trim().is_empty() {
                unused_name(base, |n| ws.contains(n) || names.iter().any(|v| v == n))
            } else {
                edited.trim().to_owned()
            };
            super::check_export_name(&name)?;
            if name == "matrust_basic_fitting"
                || !matrust::files::valid_name(&name)
                || name.starts_with(&self.prefix(ws))
            {
                return Err(fail(
                    "choose valid output names outside the private bf_ namespace",
                ));
            }
            if ws.contains(&name) || names.contains(&name) {
                return Err(fail(&format!(
                    "{name} is already used; choose an unused output name"
                )));
            }
            names.push(name);
            Ok(())
        };
        for (i, enabled) in self.enabled.iter().enumerate() {
            if *enabled {
                allocate(&self.fit_names[i], "fit")?;
            }
        }
        if !self.evaluate.trim().is_empty() {
            allocate(&self.evaluated_x_name, "x1")?;
            allocate(&self.evaluated_y_name, "f1")?;
        }
        Ok(names)
    }
    fn source(&self, ws: &Snapshot) -> Result<String, String> {
        if self.workspace_data {
            for name in [&self.x, &self.y] {
                let Some(matrust::Array::Numeric(m)) = ws.get(name) else {
                    return Err(fail("choose real workspace x and y vectors"));
                };
                if (m.rows() != 1 && m.cols() != 1) || !(2..=MAX_DATA).contains(&m.len()) {
                    return Err(fail("data must be vectors with 2 to 20000 samples"));
                }
            }
            // Insert source names after the private namespace has been allocated.
            Ok(format!("bf_x = {}; bf_y = {};\n", self.x, self.y))
        } else {
            let l = self
                .lines
                .get(self.selected_line)
                .ok_or_else(|| fail("choose a line in an open figure or workspace vectors"))?;
            if l.is_3d {
                return Err(fail("choose a two-dimensional source line"));
            }
            if !(2..=MAX_DATA).contains(&l.points.len()) {
                return Err(fail("the line needs 2 to 20000 samples"));
            }
            Ok(format!(
                "bf_figs = findobj(0, 'Type', 'figure', 'Number', {});\nif isempty(bf_figs), error('Basic Fitting: source figure is closed'); end\nbf_fig = bf_figs(1);\nbf_lines = findobj(bf_fig, 'Type', 'line');\nif numel(bf_lines) < {}, error('Basic Fitting: source line is missing'); end\nbf_source = bf_lines({}); bf_ax = get(bf_source, 'Parent');\nbf_x = get(bf_source, 'XData'); bf_y = get(bf_source, 'YData');\n",
                l.figure, l.ordinal, l.ordinal
            ))
        }
    }
    /// Minimal bounded replica, used only to seed the private Runner engine.
    fn seed_preview(&mut self) {
        if self.workspace_data {
            self.seed.clear();
            self.seeded = None;
            return;
        }
        let Some(line) = self.lines.get(self.selected_line) else {
            self.seed.clear();
            return;
        };
        if self.seeded.as_ref().is_some_and(|(id, n, p)| {
            *id == line.figure && *n == line.ordinal && p.same_storage(&line.points)
        }) {
            return;
        }
        self.seed.clear();
        if line.points.len() > MAX_DATA {
            return;
        }
        self.seed = format!("figure({});\n", line.figure);
        // Only 2-D line objects are offered. Placeholders retain findobj indexes
        // without copying any unselected line's samples.
        let mut axis = None;
        for l in self.lines.iter().filter(|l| l.figure == line.figure) {
            if axis != Some(l.axes) {
                let [left, top, right, bottom] = l.axes.rectangle();
                let rect = [left, 1. - bottom, right - left, bottom - top];
                self.seed += &format!(
                    "subplot('Position', {}); hold on;\n",
                    vector(rect.into_iter())
                );
                axis = Some(l.axes);
            }
            let (x, y) = if l.ordinal == line.ordinal {
                (
                    vector(l.points.iter().map(|p| p[0])),
                    vector(l.points.iter().map(|p| p[1])),
                )
            } else {
                ("[0 1]".into(), "[0 0]".into())
            };
            self.seed += &format!("plot({x}, {y});\n");
        }
        self.seeded = Some((line.figure, line.ordinal, line.points.clone()));
    }
}
impl App for BasicFitting {
    fn kind(&self) -> AppKind {
        AppKind::BasicFitting
    }
    fn preview_setup(&self) -> String {
        self.seed.clone()
    }
    /// Single script path for preview, Apply, export and saved scripts.
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        if !self.enabled.iter().any(|v| *v) {
            return Err(fail("select at least one fit"));
        }
        let prefix = self.prefix(ws);
        let names = self.output_names(ws)?;
        let source = self.source(ws)?;
        expression(self.evaluate.trim())?;
        let range = match (self.range_min.trim(), self.range_max.trim()) {
            ("", "") => None,
            (a, b) => match (a.parse::<f64>(), b.parse::<f64>()) {
                (Ok(a), Ok(b)) if a.is_finite() && b.is_finite() && a < b => Some((a, b)),
                _ => {
                    return Err(fail(
                        "enter a finite increasing preview x range, or leave both ends blank",
                    ));
                }
            },
        };
        let mut code = String::from(
            "% Basic Fitting: coefficients and residual norms are computed by engine builtins.\n%SOURCE%\nbf_x = bf_x(:); bf_y = bf_y(:);\nif ~isvector(bf_x) || ~isvector(bf_y) || numel(bf_x) ~= numel(bf_y) || numel(bf_x) < 2 || numel(bf_x) > 20000, error('Basic Fitting: matching vectors with 2 to 20000 samples required'); end\nif any(~isfinite(bf_x)) || any(~isfinite(bf_y)), error('Basic Fitting: data contains NaN or Inf'); end\n[bf_x, bf_order] = sort(bf_x); bf_y = bf_y(bf_order);\n",
        );
        code += &match range {
            Some((lo, hi)) => format!(
                "bf_grid = linspace(min(min(bf_x), {}), max(max(bf_x), {}), {PREVIEW_POINTS})';\n",
                number(lo),
                number(hi)
            ),
            None => format!("bf_grid = linspace(min(bf_x), max(bf_x), {PREVIEW_POINTS})';\n"),
        };
        let evaluating = !self.evaluate.trim().is_empty();
        if evaluating {
            code += &format!(
                "bf_evalx = (%EVALUATE%);\nif ~isnumeric(bf_evalx) || ~isreal(bf_evalx) || ~isvector(bf_evalx) || numel(bf_evalx) > {MAX_EVALUATE} || any(~isfinite(bf_evalx(:))), error('Basic Fitting: evaluation needs a finite real vector with at most 2000 values'); end\nbf_evalx = bf_evalx(:);\n"
            );
        } else {
            code += "bf_evalx = [];\n";
        }
        for (i, kind) in FitKind::ALL
            .into_iter()
            .enumerate()
            .filter(|(i, _)| self.enabled[*i])
        {
            let id = format!("bf_fit{i}");
            code += &format!(
                "{id} = struct('type', '{}', 'coeff', [], 'normresid', 0, 'mu', [], 'pp', []);\n",
                kind.label()
            );
            let eval = match kind {
                FitKind::Polynomial(degree) => {
                    code += &format!(
                        "if numel(unique(bf_x)) <= {degree}, error('Basic Fitting warning: too few distinct points for {}'); end\n",
                        kind.label()
                    );
                    if self.normalize {
                        code += &format!(
                            "[{id}.coeff, bf_stats{i}, {id}.mu] = polyfit(bf_x, bf_y, {degree});\n"
                        );
                        format!("polyval({id}.coeff, %X%, [], {id}.mu)")
                    } else {
                        code += &format!(
                            "[{id}.coeff, bf_stats{i}] = polyfit(bf_x, bf_y, {degree});\n"
                        );
                        format!("polyval({id}.coeff, %X%)")
                    }
                }
                FitKind::Spline | FitKind::Pchip => {
                    code += "if numel(unique(bf_x)) ~= numel(bf_x), error('Basic Fitting: interpolants need distinct x values'); end\n";
                    code += &format!(
                        "{id}.pp = {}(bf_x, bf_y); {id}.coeff = {id}.pp.coefs;\n",
                        kind.label()
                    );
                    format!("ppval({id}.pp, %X%)")
                }
            };
            code += &format!(
                "bf_at{i} = {}; bf_at{i} = bf_at{i}(:);\nbf_res{i} = bf_y - bf_at{i}; {id}.normresid = norm(bf_res{i});\nbf_curve{i} = {}; bf_curve{i} = bf_curve{i}(:);\n",
                eval.replace("%X%", "bf_x"),
                eval.replace("%X%", "bf_grid")
            );
            if evaluating && i == self.selection() {
                code += &format!(
                    "bf_evaly = {}; bf_evaly = bf_evaly(:);\n",
                    eval.replace("%X%", "bf_evalx")
                );
            }
            code += &format!(
                "if any(~isfinite(bf_curve{i})) || ~isfinite({id}.normresid), error('Basic Fitting: fit overflowed; reduce the degree or x range, or center and scale x'); end\n"
            );
            // sprintf formats the engine's values, including the normalization.
            let equation = if let FitKind::Polynomial(degree) = kind {
                let terms = (0..=degree)
                    .map(|k| {
                        format!(
                            "(%.*g)*{}^{}",
                            if self.normalize { "z" } else { "x" },
                            degree - k
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" + ");
                let mut args = (0..=degree)
                    .map(|k| format!(", {}, {id}.coeff({})", self.digits.clamp(2, 8), k + 1))
                    .collect::<String>();
                let mut template = format!("{}: y = {terms}", kind.label());
                if self.normalize {
                    template += "; z=(x-%.*g)/%.*g";
                    args += &format!(
                        ", {}, {id}.mu(1), {}, {id}.mu(2)",
                        self.digits.clamp(2, 8),
                        self.digits.clamp(2, 8)
                    );
                }
                format!("sprintf({}{args})", quote(&template))
            } else {
                quote(&format!("{} piecewise cubic", kind.label()))
            };
            code += &format!("bf_equation{i} = {equation};\n");
        }
        // All numerical validation precedes graphics changes and output writes.
        if !self.workspace_data {
            code += "if exist('matrust_basic_fitting', 'var') == 1\n\
                if ~isstruct(matrust_basic_fitting) || ~isfield(matrust_basic_fitting, 'entries') || ~iscell(matrust_basic_fitting.entries) || numel(matrust_basic_fitting.entries) > 256, error('Basic Fitting: reserved graphics registry already exists'); end\n\
                else, matrust_basic_fitting = struct(); matrust_basic_fitting.entries = {}; end\n\
                bf_entry = 0;\n\
                for bf_k = 1:numel(matrust_basic_fitting.entries)\n\
                  bf_previous = matrust_basic_fitting.entries{bf_k};\n\
                  if isgraphics(bf_previous.axes) && isequal(bf_previous.axes, bf_ax), bf_entry = bf_k; end\n\
                end\n\
                if bf_entry > 0\n\
                  bf_previous = matrust_basic_fitting.entries{bf_entry}; bf_box = bf_previous.position;\n\
                  for bf_k = 1:numel(bf_previous.handles)\n\
                    if isgraphics(bf_previous.handles{bf_k}), delete(bf_previous.handles{bf_k}); end\n\
                  end\n\
                  for bf_k = 1:numel(bf_previous.residual_axes)\n\
                    bf_owned_ax = bf_previous.residual_axes{bf_k};\n\
                    if isgraphics(bf_owned_ax) && isempty(get(bf_owned_ax, 'Children')), delete(bf_owned_ax); end\n\
                  end\n\
                else\n\
                  bf_entry = numel(matrust_basic_fitting.entries) + 1;\n\
                  if bf_entry > 256, error('Basic Fitting: graphics registry is full'); end\n\
                  bf_box = get(bf_ax, 'Position');\n\
                end\n\
                bf_state = struct(); bf_state.axes = bf_ax; bf_state.position = bf_box; bf_state.handles = {}; bf_state.residual_axes = {};\n";
            if !self.residuals || self.separate_residuals {
                code += "set(bf_ax, 'Position', bf_box, 'PositionConstraint', 'innerposition');\n";
            }
        }
        if self.workspace_data {
            code += "bf_fig = figure;\n";
            if self.residuals && !self.separate_residuals {
                code += "subplot(2,1,1);\n";
            }
            code += "bf_ax = gca; plot(bf_ax, bf_x, bf_y, 'o', 'DisplayName', 'data');\n";
        } else if self.residuals && !self.separate_residuals {
            // Split only the selected axes' box; neighbouring axes and their
            // line handles are preserved. Compute the lower box before moving.
            code += "bf_lower = [bf_box(1), bf_box(2), bf_box(3), bf_box(4)*0.32];\nfigure(bf_fig); set(bf_ax, 'Position', [bf_box(1), bf_box(2)+bf_box(4)*0.46, bf_box(3), bf_box(4)*0.54], 'PositionConstraint', 'innerposition');\n";
        }
        code += "bf_hold = get(bf_ax, 'NextPlot'); hold(bf_ax, 'on');\n";
        for (i, kind) in FitKind::ALL
            .into_iter()
            .enumerate()
            .filter(|(i, _)| self.enabled[*i])
        {
            code += &format!(
                "bf_draw = plot(bf_ax, bf_grid, bf_curve{i}, 'DisplayName', '{}');\n",
                kind.label()
            );
            if !self.workspace_data {
                code += "bf_state.handles{end+1} = bf_draw;\n";
            }
            if self.equations || self.show_norm {
                code += &format!("bf_label{i} = '';\n");
                if self.equations {
                    code += &format!("bf_label{i} = bf_equation{i};\n");
                }
                if self.show_norm {
                    code += &format!(
                        "bf_label{i} = [bf_label{i} sprintf('  {}: norm(resid) = %.*g', {}, bf_fit{i}.normresid)];\n",
                        kind.label(),
                        self.digits.clamp(2, 8)
                    );
                }
                code += &format!(
                    "bf_draw = text(bf_ax, min(bf_grid), max(bf_y) - {}*(max(bf_y)-min(bf_y)+eps), bf_label{i}, 'Interpreter', 'none');\n",
                    number(i as f64 * 0.065)
                );
                if !self.workspace_data {
                    code += "bf_state.handles{end+1} = bf_draw;\n";
                }
            }
        }
        if evaluating && self.plot_evaluated {
            code += "bf_draw = plot(bf_ax, bf_evalx, bf_evaly, 'o', 'DisplayName', 'evaluated points');\n";
            if !self.workspace_data {
                code += "bf_state.handles{end+1} = bf_draw;\n";
            }
        }
        code += "set(bf_ax, 'NextPlot', bf_hold);\n";
        if self.residuals {
            if !self.workspace_data {
                code += "bf_existing_axes = findobj(0, 'Type', 'axes');\n";
            }
            if self.separate_residuals {
                code += "figure; bf_resax = gca;\n";
            } else {
                if self.workspace_data {
                    code += "figure(bf_fig); subplot(2,1,2); bf_resax = gca;\n";
                } else {
                    code += "figure(bf_fig); subplot('Position', bf_lower); bf_resax = gca;\n";
                }
            }
            if !self.workspace_data {
                code += "bf_new_axis = true;\nfor bf_k = 1:numel(bf_existing_axes)\n  if isequal(bf_existing_axes(bf_k), bf_resax), bf_new_axis = false; end\nend\nif bf_new_axis, bf_state.residual_axes{end+1} = bf_resax; end\n";
            }
            code += "hold(bf_resax, 'on');\n";
            for (i, kind) in FitKind::ALL
                .into_iter()
                .enumerate()
                .filter(|(i, _)| self.enabled[*i])
            {
                code += &format!(
                    "bf_draw = {}(bf_resax, bf_x, bf_res{i}, 'DisplayName', '{} residuals');\n",
                    self.residual_style.command(),
                    kind.label()
                );
                if !self.workspace_data {
                    code += "bf_state.handles{end+1} = bf_draw;\n";
                }
            }
            code += "hold(bf_resax, 'off'); ylabel(bf_resax, 'Residual');\n";
        }
        if !self.workspace_data {
            code += "matrust_basic_fitting.entries{bf_entry} = bf_state;\n";
        }
        for k in 0..names.len() {
            let value = if k < self.enabled.iter().filter(|v| **v).count() {
                let i = self
                    .enabled
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| **e)
                    .nth(k)
                    .unwrap()
                    .0;
                format!("bf_fit{i}")
            } else if k == names.len() - 2 {
                "bf_evalx".into()
            } else {
                "bf_evaly".into()
            };
            code += &format!("%OUTPUT{k}% = {value};\n");
        }
        // Replace only private identifiers. External identifiers/expressions are
        // inserted afterwards so workspace variables such as bf_x remain valid.
        code = code.replace("bf_", &prefix);
        let source = if self.workspace_data {
            format!("{prefix}x = {}; {prefix}y = {};\n", self.x, self.y)
        } else {
            source.replace("bf_", &prefix)
        };
        code = code
            .replace("%SOURCE%", &source)
            .replace("%EVALUATE%", self.evaluate.trim());
        for (k, name) in names.iter().enumerate() {
            code = code.replace(&format!("%OUTPUT{k}%"), name);
        }
        let guards = names.iter().map(|name| format!(
            "if exist({}, 'var') == 1, error('Basic Fitting: output name already exists'); end\n", quote(name)
        )).collect::<String>();
        Ok(format!("{guards}{code}"))
    }
    fn inputs(&self) -> Vec<String> {
        let mut names = self.eval_inputs.clone();
        if self.workspace_data {
            names.extend([self.x.clone(), self.y.clone()]);
        }
        names.sort();
        names.dedup();
        names
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let p = self.prefix(ws);
        let mut names = vec![
            format!("{p}x"),
            format!("{p}y"),
            format!("{p}grid"),
            format!("{p}evalx"),
        ];
        let mut code = String::new();
        for (i, kind) in FitKind::ALL
            .into_iter()
            .enumerate()
            .filter(|(i, _)| self.enabled[*i])
        {
            for (suffix, expr) in [
                ("coeff", format!("{p}fit{i}.coeff")),
                ("mu", format!("{p}fit{i}.mu")),
                ("norm", format!("{p}fit{i}.normresid")),
            ] {
                let n = format!("{p}{suffix}{i}");
                code += &format!("{n} = {expr};\n");
                names.push(n);
            }
            for suffix in ["curve", "res"] {
                names.push(format!("{p}{suffix}{i}"));
            }
            let (breaks, coefs) = if matches!(kind, FitKind::Polynomial(_)) {
                ("0".into(), "0".into())
            } else {
                (
                    format!("numel({p}fit{i}.pp.breaks)"),
                    format!("numel({p}fit{i}.pp.coefs)"),
                )
            };
            for (suffix, value) in [("breaks", breaks), ("count", coefs)] {
                let n = format!("{p}{suffix}{i}");
                code += &format!("{n} = {value};\n");
                names.push(n);
            }
        }
        if !self.evaluate.trim().is_empty() {
            names.push(format!("{p}evaly"));
        }
        (code, names)
    }
    fn prepare(&mut self, ws: &Snapshot) {
        let identifiers = self
            .evaluate
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .collect::<Vec<_>>();
        self.eval_inputs = ws
            .variables()
            .filter(|(n, _)| identifiers.contains(n))
            .map(|(n, _)| n.to_owned())
            .collect();
        self.evaluate_fit = self.selection();
        self.seed_preview();
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        // Probe names have an unused private prefix; infer it from the x probe.
        let p = outputs
            .keys()
            .find_map(|n| n.strip_suffix("grid"))
            .ok_or_else(|| fail("missing preview grid"))?;
        let x = real_values(outputs, &format!("{p}x"))?;
        let y = real_values(outputs, &format!("{p}y"))?;
        let grid = real_values(outputs, &format!("{p}grid"))?;
        let evalx = real_values(outputs, &format!("{p}evalx"))?;
        let mut fits = Vec::new();
        for (i, kind) in FitKind::ALL
            .into_iter()
            .enumerate()
            .filter(|(i, _)| self.enabled[*i])
        {
            let values = |suffix: &str| real_values(outputs, &format!("{p}{suffix}{i}"));
            let scalar = |suffix: &str| -> Result<f64, String> {
                values(suffix)?
                    .first()
                    .copied()
                    .ok_or_else(|| fail("empty result"))
            };
            let coefficients = values("coeff")?;
            let mu = values("mu")?;
            let equation = formula(kind, &coefficients, &mu, self.digits);
            let evaluated = if i == self.selection() && !evalx.is_empty() {
                pairs(&evalx, &real_values(outputs, &format!("{p}evaly"))?)
            } else {
                Vec::new()
            };
            fits.push(FitResult {
                kind,
                coefficients,
                mu,
                norm: scalar("norm")?,
                breaks: scalar("breaks")? as usize,
                coefficient_count: scalar("count")? as usize,
                curve: pairs(&grid, &values("curve")?),
                residuals: pairs(&x, &values("res")?),
                evaluated,
                formula: equation,
            });
        }
        self.current = Some(ResultSet {
            data: pairs(&x, &y),
            fits,
        });
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        field(ui, "Data", |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.workspace_data, false, "Figure line");
                ui.selectable_value(&mut self.workspace_data, true, "Workspace");
            });
        });
        if self.workspace_data {
            for (label, selected) in [("x vector", &mut self.x), ("y vector", &mut self.y)] {
                field(ui, label, |ui| {
                    egui::ComboBox::from_id_salt(("basic_fitting_vector", label))
                        .selected_text(if selected.is_empty() {
                            "Choose vector"
                        } else {
                            selected.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for (n, a) in ws.variables() {
                                if let matrust::Array::Numeric(m) = a
                                    && (m.rows() == 1 || m.cols() == 1)
                                    && (2..=MAX_DATA).contains(&m.len())
                                {
                                    ui.selectable_value(selected, n.to_owned(), n);
                                }
                            }
                        });
                });
            }
        } else {
            egui::ComboBox::from_id_salt("basic_fitting_line")
                .width(ui.available_width())
                .selected_text(
                    self.lines
                        .get(self.selected_line)
                        .map_or("Choose line".into(), LineChoice::caption),
                )
                .show_ui(ui, |ui| {
                    for (i, line) in self.lines.iter().enumerate() {
                        ui.selectable_value(&mut self.selected_line, i, line.caption());
                    }
                });
            super::field(ui, "", |ui| {
                crate::tokens::helper(ui, "Open figure lines refresh automatically.");
            });
        }
        ui.separator();
        ui.label("Fits");
        for (i, kind) in FitKind::ALL.into_iter().enumerate() {
            ui.checkbox(
                &mut self.enabled[i],
                match kind {
                    FitKind::Spline => "Spline interpolant",
                    FitKind::Pchip => "Shape-preserving (pchip)",
                    _ => kind.label(),
                },
            );
        }
        ui.separator();
        ui.checkbox(&mut self.normalize, "Center and scale x (mu)");
        ui.horizontal(|ui| {
            ui.label("Significant digits");
            ui.add(egui::DragValue::new(&mut self.digits).range(2..=8));
        });
        ui.checkbox(&mut self.equations, "Show equations on plot");
        ui.checkbox(&mut self.show_norm, "Show norm of residuals");
        ui.checkbox(&mut self.residuals, "Plot residuals");
        if self.residuals {
            ui.horizontal(|ui| {
                for (style, label) in [
                    (ResidualStyle::Bar, "Bar"),
                    (ResidualStyle::Scatter, "Scatter"),
                    (ResidualStyle::Line, "Line"),
                ] {
                    ui.selectable_value(&mut self.residual_style, style, label);
                }
            });
            ui.checkbox(&mut self.separate_residuals, "Separate residual figure");
        }
        field(ui, "Preview x range (blank uses data)", |ui| {
            ui.horizontal(|ui| {
                let w = (ui.available_width() - crate::tokens::S) / 2.;
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.range_min).char_limit(32),
                    )
                    .hint_text("minimum")
                    .desired_width(w),
                );
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.range_max).char_limit(32),
                    )
                    .hint_text("maximum")
                    .desired_width(w),
                );
            });
        });
    }
    fn main_caption(&self) -> &'static str {
        "PREVIEW"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_enabled(self.can_apply, egui::Button::new("Apply to figure"))
            .clicked()
        {
            self.apply_requested = true;
        }
        let Some(current) = &self.current else {
            crate::tokens::helper(
                ui,
                "Choose data and fits. The status line shows input errors.",
            );
            return;
        };
        let height = (ui.available_height() - crate::tokens::S).max(180.);
        plot(
            "basic_fitting_preview",
            if self.residuals {
                height * 0.64
            } else {
                height
            },
        )
        .show(ui, |p| {
            p.points(egui_plot::Points::new("data", display_points(&current.data)).radius(2.5));
            for fit in &current.fits {
                p.line(egui_plot::Line::new(
                    fit.kind.label(),
                    display_points(&fit.curve),
                ));
                if self.plot_evaluated && !fit.evaluated.is_empty() {
                    p.points(
                        egui_plot::Points::new("evaluated points", fit.evaluated.clone())
                            .radius(4.),
                    );
                }
                if self.equations || self.show_norm {
                    let label = format!(
                        "{}{}",
                        if self.equations {
                            fit.formula.clone()
                        } else {
                            fit.kind.label().to_owned()
                        },
                        if self.show_norm {
                            format!("; norm(resid) = {}", significant(fit.norm, self.digits))
                        } else {
                            String::new()
                        }
                    );
                    if let Some(pt) = fit.curve.get(fit.curve.len() / 2) {
                        p.text(egui_plot::Text::new(
                            fit.kind.label(),
                            egui_plot::PlotPoint::new(pt[0], pt[1]),
                            label,
                        ));
                    }
                }
            }
        });
        if self.residuals {
            plot("basic_fitting_residuals", height * 0.32)
                .y_axis_label("Residual")
                .show(ui, |p| {
                    p.hline(egui_plot::HLine::new("zero", 0.));
                    for fit in &current.fits {
                        match self.residual_style {
                            ResidualStyle::Scatter => p.points(
                                egui_plot::Points::new(
                                    fit.kind.label(),
                                    display_points(&fit.residuals),
                                )
                                .radius(2.),
                            ),
                            ResidualStyle::Line => p.line(egui_plot::Line::new(
                                fit.kind.label(),
                                display_points(&fit.residuals),
                            )),
                            ResidualStyle::Bar => p.bar_chart(egui_plot::BarChart::new(
                                fit.kind.label(),
                                display_points(&fit.residuals)
                                    .iter()
                                    .map(|p| egui_plot::Bar::new(p[0], p[1]))
                                    .collect(),
                            )),
                        }
                    }
                });
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let mut rows = Vec::new();
        if let Some(current) = &self.current {
            for fit in &current.fits {
                rows.push(("Fit", fit.kind.label().into()));
                rows.push((
                    "Formula",
                    formula(fit.kind, &fit.coefficients, &fit.mu, self.digits),
                ));
                if matches!(fit.kind, FitKind::Polynomial(_)) {
                    rows.push((
                        "Coefficients",
                        fit.coefficients
                            .iter()
                            .map(|v| significant(*v, self.digits))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ));
                } else {
                    rows.push((
                        "Breaks / coefs",
                        format!("{} / {}", fit.breaks, fit.coefficient_count),
                    ));
                }
                if self.show_norm {
                    rows.push(("norm(resid)", significant(fit.norm, self.digits)));
                }
            }
        }
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        field(ui, "Evaluate at x", |ui| {
            ui.add(
                crate::tokens::input(
                    egui::TextEdit::singleline(&mut self.evaluate).char_limit(256),
                )
                .hint_text("0:0.1:10")
                .desired_width(ui.available_width()),
            );
            super::expression_message(ui, expression(self.evaluate.trim()));
            egui::ComboBox::from_id_salt("basic_fitting_eval_fit")
                .selected_text(FitKind::ALL[self.selection()].label())
                .show_ui(ui, |ui| {
                    for (i, k) in FitKind::ALL
                        .into_iter()
                        .enumerate()
                        .filter(|(i, _)| self.enabled[*i])
                    {
                        ui.selectable_value(&mut self.evaluate_fit, i, k.label());
                    }
                });
            ui.checkbox(&mut self.plot_evaluated, "Plot evaluated points");
        });
        if let Some(current) = &self.current {
            for fit in &current.fits {
                if !fit.evaluated.is_empty() {
                    crate::tokens::helper(ui, format!("{} evaluated values", fit.kind.label()));
                    egui::ScrollArea::vertical()
                        .id_salt("basic_fitting_table")
                        .max_height(180.)
                        .show(ui, |ui| {
                            egui::Grid::new("basic_fitting_values").show(ui, |ui| {
                                ui.label("x");
                                ui.label("f(x)");
                                ui.end_row();
                                for p in &fit.evaluated {
                                    ui.label(significant(p[0], self.digits));
                                    ui.label(significant(p[1], self.digits));
                                    ui.end_row();
                                }
                            });
                        });
                }
            }
        }
        ui.separator();
        let defaults = self.output_names(ws).unwrap_or_default();
        let mut n = 0;
        for (i, k) in FitKind::ALL
            .into_iter()
            .enumerate()
            .filter(|(i, _)| self.enabled[*i])
        {
            output_name(
                ui,
                &format!("{} struct", k.label()),
                &mut self.fit_names[i],
                defaults.get(n),
            );
            n += 1;
        }
        if !self.evaluate.trim().is_empty() {
            output_name(
                ui,
                "Evaluation x",
                &mut self.evaluated_x_name,
                defaults.get(n),
            );
            output_name(
                ui,
                "Evaluation values",
                &mut self.evaluated_y_name,
                defaults.get(n + 1),
            );
        }
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        self.output_names(ws).unwrap_or_default()
    }
    fn summary(&self) -> Option<String> {
        self.current.as_ref().map(|r| {
            format!(
                "Basic Fitting: {} fits, {} data points",
                r.fits.len(),
                r.data.len()
            )
        })
    }
}
fn output_name(ui: &mut egui::Ui, label: &str, value: &mut String, default: Option<&String>) {
    field(ui, label, |ui| {
        ui.add(
            crate::tokens::input(egui::TextEdit::singleline(value).char_limit(63))
                .hint_text(default.map_or("unused name", String::as_str))
                .desired_width(ui.available_width()),
        );
    });
}
/// Bound drawing work independently of the full data used by the engine.
fn display_points(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let step = points.len().div_ceil(2_000).max(1);
    let mut shown = points.iter().step_by(step).copied().collect::<Vec<_>>();
    if let Some(last) = points.last()
        && shown.last() != Some(last)
    {
        shown.push(*last);
    }
    shown
}
fn plot(id: &str, height: f32) -> egui_plot::Plot<'_> {
    egui_plot::Plot::new(id)
        .height(height)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .show_background(false)
        .legend(egui_plot::Legend::default())
}
/// The Apply button uses the same guarded, transactional script as Export.
pub(super) fn show(
    host: &mut Host<BasicFitting>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    host.app.can_apply = host.ready(ws, busy)
        && host
            .request(ws)
            .is_ok_and(|r| Some(r.key) == host.runner.key());
    host.show(ui, ws, busy, action);
    if std::mem::take(&mut host.app.apply_requested)
        && host.ready(ws, busy)
        && host
            .request(ws)
            .is_ok_and(|r| Some(r.key) == host.runner.key())
    {
        *action = host.export(ws);
        host.message = "Applied fits to the figure".into();
    }
}
