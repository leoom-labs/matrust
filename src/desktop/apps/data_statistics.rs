//! Statistics of a plotted line or workspace vectors. The same script computes
//! the table, the preview, exports and saved scripts; the UI does no statistics.
//! Low-level `line` is the engine's supported alternative to xline/yline.
use super::{App, AppKind, Host, Outputs, Snapshot, field, number, short, unused_name};
use crate::toolkits::Action;
use eframe::egui;
use matrust::plot::{Figure, PlotKind, SeriesPoints};
use std::collections::BTreeMap;

pub const FIELDS: [&str; 7] = ["min", "max", "mean", "median", "mode", "std", "range"];
pub const MAX_POINTS: usize = 100_000;
/// Literal fallback for a figure must fit comfortably in the expression budget.
pub const MAX_FIGURE_POINTS: usize = 10_000;
const STATE: &str = "matrust_data_statistics";
const TAG: &str = "MatRust Data Statistics v1";
const FUNCTION: &str = "matrust_data_statistics_run";

fn reserved_name(name: &str) -> bool {
    name == STATE
        || name == FUNCTION
        || name == "matrust_export_error"
        || name.starts_with("mrexp_")
        || name.starts_with("app_ds_")
}

fn quote(s: &str) -> String {
    if s.chars().any(char::is_control) {
        format!(
            "char([{}])",
            s.encode_utf16()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        )
    } else {
        format!("'{}'", s.replace('\'', "''"))
    }
}
fn literal(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.is_infinite() {
        if v > 0. { "Inf" } else { "-Inf" }.into()
    } else {
        number(v)
    }
}
fn vector(values: impl Iterator<Item = f64>) -> String {
    format!("[{}]", values.map(literal).collect::<Vec<_>>().join(" "))
}
fn values(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(matrust::Array::Numeric(m)) => Ok(m.data().to_vec()),
        _ => Err(format!("Data Statistics: {name} is not a real array")),
    }
}
fn vector_length(array: &matrust::Array) -> Option<usize> {
    let dims = array.dims();
    let real = matches!(array, matrust::Array::Numeric(_) | matrust::Array::Typed(_))
        && !array.is_complex();
    (real && dims.len() == 2 && (dims[0] <= 1 || dims[1] <= 1)).then(|| array.len())
}

fn workspace_vectors(ws: &Snapshot) -> Vec<(String, usize)> {
    ws.variables()
        .filter_map(|(name, array)| vector_length(array).map(|n| (name.to_owned(), n)))
        .collect()
}

pub struct Source {
    pub figure: u32,
    pub axes: usize,
    position: [f64; 4],
    pub label: String,
    occurrence: usize,
    identity: matrust::ObjectIdentity,
    pub points: SeriesPoints,
    snapshot_literals: std::sync::OnceLock<[String; 2]>,
}
impl Source {
    pub fn label(&self) -> String {
        format!(
            "Figure {} · Axes {} · {}",
            self.figure,
            self.axes,
            if self.label.is_empty() {
                "Unnamed line"
            } else {
                &self.label
            }
        )
    }
}

#[derive(Clone, Debug)]
pub struct Statistics {
    pub x: [f64; 7],
    pub y: [f64; 7],
    pub points: Vec<egui_plot::PlotPoint>,
    pub overlays: Vec<([f64; 2], [f64; 2])>,
    pub names: Vec<String>,
    pub count: usize,
    pub ignored: [usize; 2],
}

#[derive(Default)]
pub struct DataStatistics {
    pub workspace: bool,
    pub x_variable: String,
    pub y_variable: String,
    pub sources: Vec<Source>,
    pub selected: usize,
    pub show: [[bool; 7]; 2],
    pub x_name: String,
    pub y_name: String,
    pub x_edited: bool,
    pub y_edited: bool,
    pub current: Option<Statistics>,
    figures: BTreeMap<u32, Figure>,
    /// Once applied, checkbox changes update the source figure as well.
    live_apply: bool,
    pending_apply: bool,
}
impl DataStatistics {
    /// Metadata and shared points only. A changed figure invalidates the script
    /// key naturally; selecting another axes does not lose the selected line.
    pub fn sync_figures(&mut self, figures: &BTreeMap<u32, Figure>) {
        if self.figures.len() == figures.len()
            && self
                .figures
                .iter()
                .zip(figures)
                .all(|((n, old), (m, new))| n == m && old.same_snapshot(new))
        {
            return;
        }
        let selected = self
            .sources
            .get(self.selected)
            .map(|s| (s.identity.clone(), s.points.clone()));
        self.sources.clear();
        for (&figure, f) in figures {
            let positions = f.normalized_axes_positions();
            for (axes_index, (pos, axes)) in f.all_axes().enumerate() {
                let mut occurrences = BTreeMap::<String, usize>::new();
                for line in &axes.series {
                    if line.kind != PlotKind::Line || !line.z.is_empty() {
                        continue;
                    }
                    let occurrence = occurrences.entry(line.label.clone()).or_default();
                    *occurrence += 1;
                    self.sources.push(Source {
                        figure,
                        axes: axes_index + 1,
                        position: positions.iter().find(|(p, _)| *p == pos).unwrap().1,
                        label: line.label.clone(),
                        occurrence: *occurrence,
                        identity: line.identity.clone(),
                        points: line.points.clone(),
                        snapshot_literals: std::sync::OnceLock::new(),
                    });
                }
            }
        }
        self.selected = match selected {
            Some((id, points)) => match self
                .sources
                .iter()
                .position(|s| s.identity.same_object(&id))
            {
                Some(index) => {
                    if !self.sources[index].points.same_storage(&points) {
                        self.pending_apply |= self.live_apply;
                    }
                    index
                }
                None => {
                    self.live_apply = false;
                    self.pending_apply = false;
                    0
                }
            },
            None => 0,
        };
        self.figures = figures.clone();
    }
    pub fn names(&self, ws: &Snapshot) -> Result<[String; 2], String> {
        let x = if self.x_edited {
            self.x_name.trim().into()
        } else {
            unused_name("xstats", |n| ws.contains(n))
        };
        let y = if self.y_edited {
            self.y_name.trim().into()
        } else {
            unused_name("ystats", |n| ws.contains(n) || n == x)
        };
        for n in [&x, &y] {
            super::check_export_name(n)?;
            if crate::workspace_edit::validate_name(n).is_err() || reserved_name(n) {
                return Err("Data Statistics: use valid, unreserved output names".into());
            }
            if ws.contains(n) {
                return Err(format!(
                    "Data Statistics: {n} already exists; choose an unused name"
                ));
            }
        }
        if x == y {
            return Err("Data Statistics: X and Y outputs need different names".into());
        }
        Ok([x, y])
    }
    fn data_code(&self, ws: &Snapshot) -> Result<String, String> {
        if self.workspace {
            for name in [&self.x_variable, &self.y_variable] {
                if reserved_name(name) {
                    return Err(
                        "Data Statistics: choose input names outside the reserved app namespace"
                            .into(),
                    );
                }
                let length = ws
                    .get(name)
                    .and_then(vector_length)
                    .ok_or("Data Statistics: choose real workspace X and Y vectors")?;
                if length > MAX_POINTS {
                    return Err(format!("Data Statistics: at most {MAX_POINTS} points"));
                }
            }
            return Ok(String::from("x = double(x); y = double(y);\n"));
        }
        let source = self
            .sources
            .get(self.selected)
            .ok_or("Data Statistics: choose an open figure with a 2-D line")?;
        if source.points.is_empty() {
            return Err("Data Statistics: choose a nonempty line".into());
        }
        if source.points.len() > MAX_FIGURE_POINTS {
            return Err(format!(
                "Data Statistics: figure input supports at most {MAX_FIGURE_POINTS} points; use workspace vectors for larger data"
            ));
        }
        let [x, y] = source.snapshot_literals.get_or_init(|| {
            [
                vector(source.points.iter().map(|p| p[0])),
                vector(source.points.iter().map(|p| p[1])),
            ]
        });
        let position = vector(source.position.into_iter());
        let label = quote(&source.label);
        // Match axes position and oldest-to-newest occurrence of DisplayName,
        // not the currently selected axes or an unstable child-array index.
        Ok(format!(
            "f = figure({figure}); aa = get(f, 'Children'); ax = [];\n\
             for k = 1:numel(aa)\n\
               if strcmp(get(aa(k), 'Type'), 'axes') && max(abs(get(aa(k), 'Position') - {position})) < 1e-10, ax = aa(k); end\n\
             end\n\
             if isempty(ax), ax = axes('Parent', f, 'Position', {position}); end\n\
             cc = get(ax, 'Children'); source = []; occurrence = 0;\n\
             for k = numel(cc):-1:1\n\
               if strcmp(get(cc(k), 'Type'), 'line') && strcmp(get(cc(k), 'DisplayName'), {label})\n\
                 occurrence = occurrence + 1;\n\
                 if occurrence == {occurrence}, source = cc(k); end\n\
               end\n\
             end\n\
             if isempty(source)\n\
               % Snapshot fallback makes the saved script reproducible in a fresh engine.\n\
               source = line(ax, {x}, {y}, 'DisplayName', {label});\n\
             end\n\
             x = get(source, 'XData'); y = get(source, 'YData');\n",
            figure = source.figure,
            occurrence = source.occurrence,
            x = x,
            y = y,
        ))
    }
    /// One builder for every destination. Applying discards the struct outputs;
    /// its local computation and drawing are identical to preview/export/save.
    fn build_script(&self, ws: &Snapshot, apply: bool) -> Result<String, String> {
        let [xname, yname] = self.names(ws)?;
        let data = self.data_code(ws)?;
        let outputs = if apply {
            "~, ~".into()
        } else {
            format!("{xname}, {yname}")
        };
        let mut code = format!(
            "% Data Statistics: NaNs omitted independently in X and Y; std uses N-1.\n\
             if exist('{STATE}', 'var') == 1\n\
               if ~isstruct({STATE}) || ~isfield({STATE}, 'tag') || ~strcmp({STATE}.tag, '{TAG}')\n\
                 error('Data Statistics: reserved registry {STATE} already exists');\n\
               end\n\
             else\n\
               {STATE} = struct(); {STATE}.tag = '{TAG}'; {STATE}.handles = {{}};\n\
             end\n"
        );
        if !apply {
            code += &format!(
                "if exist('{}', 'var') == 1 || exist('{}', 'var') == 1, error('Data Statistics: output name already exists'); end\n",
                xname, yname
            );
        }
        let arguments = if self.workspace {
            format!(", {}, {}", self.x_variable, self.y_variable)
        } else {
            String::new()
        };
        let parameters = if self.workspace { ", x, y" } else { "" };
        code += &format!(
            "[{outputs}, {STATE}] = {FUNCTION}({STATE}{arguments});\nfunction [xs, ys, state] = {FUNCTION}(state{parameters})\n{data}"
        );
        code += &format!(
            "x = x(:); y = y(:);\n\
             if isempty(x) || numel(x) ~= numel(y), error('Data Statistics: nonempty X and Y must have equal lengths'); end\n\
             if numel(x) > {MAX_POINTS}, error('Data Statistics: too many points'); end\n\
             xx = x(~isnan(x)); yy = y(~isnan(y));\n\
             if isempty(xx) || isempty(yy), error('Data Statistics: each coordinate needs a non-NaN value'); end\n"
        );
        for (axis, input) in [("xs", "xx"), ("ys", "yy")] {
            code += &format!("{axis} = struct();\n");
            for field in FIELDS {
                let args = match field {
                    "median" => format!("{input}, 'omitnan'"),
                    "std" => format!("{input}, 0, 'omitnan'"),
                    _ => input.into(),
                };
                code += &format!("{axis}.{field} = {field}({args});\n");
            }
        }
        // Workspace mode creates a new figure; figure mode targets the chosen axes.
        if self.workspace {
            code += "figure; ax = gca; line(ax, x, y, 'DisplayName', 'Data');\n";
        }
        // All validation and math precede removal of any existing app overlay.
        code += "for k = 1:numel(state.handles)\n  if isgraphics(state.handles{k}), delete(state.handles{k}); end\nend\nstate.handles = {}; state.overlay = zeros(0,4); state.names = {};\nstate.x = x; state.y = y; state.count = numel(x);\nstate.ignored = [sum(isnan(x)) sum(isnan(y))];\nxl = get(ax, 'XLim'); yl = get(ax, 'YLim');\n";
        for (a, axis) in ["x", "y"].iter().enumerate() {
            for (i, field) in FIELDS.iter().enumerate() {
                if !self.show[a][i] {
                    continue;
                }
                let levels = if *field == "std" {
                    vec![
                        (
                            format!("{axis}s.mean - {axis}s.std"),
                            format!("{axis} std -"),
                        ),
                        (
                            format!("{axis}s.mean + {axis}s.std"),
                            format!("{axis} std +"),
                        ),
                    ]
                } else {
                    vec![(format!("{axis}s.{field}"), format!("{axis} {field}"))]
                };
                for (value, name) in levels {
                    let coords = if a == 0 {
                        "[v yl(1) v yl(2)]"
                    } else {
                        "[xl(1) v xl(2) v]"
                    };
                    code += &format!(
                        "v = {value};\nif isfinite(v)\n  p = {coords};\n\
                         h = line(ax, p([1 3]), p([2 4]), 'DisplayName', '{}', 'LineStyle', '--');\n\
                         % Native Tag when supported; the registry retains exact handles otherwise.\n\
                         try, set(h, 'Tag', '{TAG}'); catch, end\n\
                         state.handles{{end+1}} = h; state.overlay(end+1,:) = p; state.names{{end+1}} = '{}';\nend\n",
                        name, name
                    );
                }
            }
        }
        code += "end\n";
        Ok(code)
    }
    pub fn apply_script(&self, ws: &Snapshot) -> Result<String, String> {
        self.build_script(ws, true)
    }
    pub fn apply(&mut self) {
        self.live_apply = true;
        self.pending_apply = true;
    }
    pub fn set_show(&mut self, axis: usize, field: usize, show: bool) {
        if self.show[axis][field] != show {
            self.show[axis][field] = show;
            self.pending_apply |= self.live_apply;
        }
    }
}

impl App for DataStatistics {
    fn kind(&self) -> AppKind {
        AppKind::DataStatistics
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        self.build_script(ws, false)
    }
    fn inputs(&self) -> Vec<String> {
        if self.workspace {
            vec![self.x_variable.clone(), self.y_variable.clone()]
        } else {
            Vec::new()
        }
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok([x, y]) = self.names(ws) else {
            return (String::new(), Vec::new());
        };
        let fields = |name: &str| {
            FIELDS
                .iter()
                .map(|f| format!("{name}.{f}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let code = format!(
            "app_ds_xs = [{}]; app_ds_ys = [{}];\napp_ds_x = {STATE}.x; app_ds_y = {STATE}.y;\napp_ds_count = {STATE}.count; app_ds_ignored = {STATE}.ignored; app_ds_overlay = {STATE}.overlay; app_ds_names = {STATE}.names;\n",
            fields(&x),
            fields(&y)
        );
        (
            code,
            ["xs", "ys", "x", "y", "count", "ignored", "overlay", "names"]
                .map(|n| format!("app_ds_{n}"))
                .into(),
        )
    }
    fn prepare(&mut self, ws: &Snapshot) {
        let vectors = workspace_vectors(ws);
        if self.x_variable.is_empty()
            && let Some((n, _)) = vectors.first()
        {
            self.x_variable = n.clone();
        }
        if self.y_variable.is_empty()
            && let Some((n, _)) = vectors.get(1).or(vectors.first())
        {
            self.y_variable = n.clone();
        }
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let x = values(outputs, "app_ds_x")?;
        let y = values(outputs, "app_ds_y")?;
        let overlay = values(outputs, "app_ds_overlay")?;
        let n = overlay.len() / 4;
        let ignored = values(outputs, "app_ds_ignored")?;
        let names = match outputs.get("app_ds_names") {
            Some(matrust::Array::Cell(c)) => (0..c.len())
                .map(|i| {
                    c.get_linear(i)
                        .ok()
                        .and_then(|v| match v.as_ref() {
                            matrust::Array::Character(c) => {
                                Some(String::from_utf16_lossy(c.units()))
                            }
                            _ => None,
                        })
                        .unwrap_or_default()
                })
                .collect(),
            _ => Vec::new(),
        };
        let result = Statistics {
            x: values(outputs, "app_ds_xs")?
                .try_into()
                .map_err(|_| "Invalid X statistics")?,
            y: values(outputs, "app_ds_ys")?
                .try_into()
                .map_err(|_| "Invalid Y statistics")?,
            points: x
                .into_iter()
                .zip(y)
                .map(|(x, y)| egui_plot::PlotPoint::new(x, y))
                .collect(),
            overlays: (0..n)
                .map(|i| {
                    (
                        [overlay[i], overlay[i + n]],
                        [overlay[i + 2 * n], overlay[i + 3 * n]],
                    )
                })
                .collect(),
            names,
            count: values(outputs, "app_ds_count")?[0] as usize,
            ignored: [ignored[0] as usize, ignored[1] as usize],
        };
        self.current = Some(result);
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        field(ui, "Data", |ui| {
            let old = self.workspace;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.workspace, false, "Figure line");
                ui.selectable_value(&mut self.workspace, true, "Workspace");
            });
            if old != self.workspace {
                self.live_apply = false;
                self.pending_apply = false;
            }
        });
        if self.workspace {
            let vectors = workspace_vectors(ws);
            for (label, chosen) in [
                ("X vector", &mut self.x_variable),
                ("Y vector", &mut self.y_variable),
            ] {
                field(ui, label, |ui| {
                    egui::ComboBox::from_id_salt(("data_statistics", label))
                        .width(ui.available_width())
                        .selected_text(if chosen.is_empty() {
                            "Choose a vector"
                        } else {
                            chosen.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for (name, n) in &vectors {
                                ui.selectable_value(chosen, name.clone(), format!("{name} ({n})"));
                            }
                        });
                });
            }
        } else {
            field(ui, "Figure · axes · DisplayName", |ui| {
                let shown = self
                    .sources
                    .get(self.selected)
                    .map_or("Choose a line".into(), Source::label);
                let old = self.selected;
                egui::ComboBox::from_id_salt("data_statistics_source")
                    .width(ui.available_width())
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for (i, source) in self.sources.iter().enumerate() {
                            ui.selectable_value(&mut self.selected, i, source.label());
                        }
                    });
                if old != self.selected {
                    self.live_apply = false;
                    self.pending_apply = false;
                }
                if self.sources.is_empty() {
                    crate::tokens::helper(ui, "No 2-D lines in open figures");
                }
            });
        }
        crate::tokens::helper(
            ui,
            "NaNs omitted independently in X and Y. Standard deviation uses N−1.",
        );
        if let Some(r) = &self.current {
            crate::tokens::helper(
                ui,
                format!(
                    "{} points · NaNs ignored: X {}, Y {}",
                    r.count, r.ignored[0], r.ignored[1]
                ),
            );
        }
    }
    fn main_caption(&self) -> &'static str {
        "STATISTICS"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("data_statistics_table")
            .num_columns(3)
            .show(ui, |ui| {
                ui.label("Statistic");
                ui.label("X");
                ui.label("Y");
                ui.end_row();
                for (i, field) in FIELDS.iter().enumerate() {
                    ui.label(*field);
                    for a in 0..2 {
                        let value = self
                            .current
                            .as_ref()
                            .map(|r| if a == 0 { r.x[i] } else { r.y[i] });
                        ui.horizontal(|ui| {
                            ui.label(value.map_or("—".into(), short));
                            let mut show = self.show[a][i];
                            if ui.checkbox(&mut show, "show on plot").changed() {
                                self.set_show(a, i, show);
                            }
                        });
                    }
                    ui.end_row();
                }
            });
        if ui
            .add_enabled(
                !self.workspace && self.current.is_some(),
                egui::Button::new("Apply to figure"),
            )
            .clicked()
        {
            self.apply();
        }
        ui.add_space(crate::tokens::S);
        if let Some(r) = &self.current {
            egui_plot::Plot::new("data_statistics_preview")
                .height(ui.available_height().max(140.))
                .allow_scroll(false)
                .legend(egui_plot::Legend::default())
                .show(ui, |plot| {
                    let [red, green, blue] = matrust::plot::COLORS[0];
                    let ink = egui::Color32::from_rgb(red, green, blue);
                    // Missing coordinates break the trace, without joining across
                    // a gap or sending NaNs to the painter. Slices stay borrowed.
                    for (i, run) in r
                        .points
                        .split(|p| !p.x.is_finite() || !p.y.is_finite())
                        .filter(|run| !run.is_empty())
                        .enumerate()
                    {
                        plot.line(
                            egui_plot::Line::new(if i == 0 { "Data" } else { "" }, run)
                                .id(egui::Id::new(("data_statistics_trace", i)))
                                .color(ink),
                        );
                    }
                    for ((a, b), name) in r.overlays.iter().zip(&r.names) {
                        plot.line(
                            egui_plot::Line::new(name.clone(), vec![*a, *b])
                                .style(egui_plot::LineStyle::dashed_dense()),
                        );
                    }
                });
        } else {
            crate::tokens::helper(ui, "Choose data to compute statistics");
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        self.current.as_ref().map_or_else(Vec::new, |r| {
            vec![
                ("Number of points", r.count.to_string()),
                ("NaNs ignored (X)", r.ignored[0].to_string()),
                ("NaNs ignored (Y)", r.ignored[1].to_string()),
            ]
        })
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        if !self.x_edited {
            self.x_name = unused_name("xstats", |n| ws.contains(n));
        }
        if !self.y_edited {
            self.y_name = unused_name("ystats", |n| ws.contains(n) || n == self.x_name);
        }
        for (label, name, edited) in [
            ("X statistics struct", &mut self.x_name, &mut self.x_edited),
            ("Y statistics struct", &mut self.y_name, &mut self.y_edited),
        ] {
            field(ui, label, |ui| {
                *edited |= ui
                    .add(
                        crate::tokens::input(egui::TextEdit::singleline(name).char_limit(63))
                            .desired_width(ui.available_width()),
                    )
                    .changed();
            });
        }
        crate::tokens::helper(ui, "Choose unused names; existing variables are preserved.");
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        self.names(ws).map_or_else(|_| Vec::new(), |n| n.into())
    }
    fn summary(&self) -> Option<String> {
        self.current.as_ref().map(|r| {
            format!(
                "Data Statistics: {} points; NaNs ignored X {}, Y {}",
                r.count, r.ignored[0], r.ignored[1]
            )
        })
    }
}

/// App-specific Apply action, routed through the workspace worker. Host still
/// owns the standard preview, export, script saving, layout and status line.
pub(super) fn show(
    host: &mut Host<DataStatistics>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    host.show(ui, ws, busy, action);
    if host.app.pending_apply
        && action.is_none()
        && host.ready(ws, busy)
        && host
            .request(ws)
            .is_ok_and(|r| Some(r.key) == host.runner.key())
    {
        match host.app.apply_script(ws) {
            Ok(script) => {
                *action = Some(Action::Script(script));
                host.app.pending_apply = false;
                host.message = "Applied statistics to the source figure".into();
            }
            Err(error) => host.script_error = Some(error),
        }
    }
}
