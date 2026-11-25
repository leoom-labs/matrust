//! PDE Modeler desktop app (8c1/8c2/8c3): interactive 2-D CSG drawing canvas,
//! editable shapes/set formula/BCs/PDE/mesh panels, Geometry/Mesh/Solution
//! figure views, background meshing/solving with cancellation, time slider and
//! eigenmode picker, DOF guard, workspace export, and `createpde` scripts.
use super::{App, AppKind, Host, Outputs, Request, Runner, Snapshot};
use crate::{theme::ThemedScrollArea, tokens, toolkits::Action};
use eframe::egui;
use matrust::{
    Array, Matrix,
    engine::{
        pde_app_model::{BoundaryCondition, GeometricOrder, PdeAppModel, PdeType, Shape},
        pde_modeler_app_hooks::Launch,
    },
};
use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

#[path = "pde_modeler/canvas.rs"]
pub(crate) mod canvas;
#[path = "pde_modeler/views.rs"]
pub(crate) mod views;

use views::{MainTab, PreparedFigure, RunKind, RunPayload, SolutionPlotOptions, TIME_DOF_LIMIT};

pub(crate) type OpenSlot = Arc<Mutex<Option<Launch>>>;

/// Minimum readable size (>= 12 px) for secondary captions and field labels.
const LABEL_SIZE: f32 = tokens::SMALL_CODE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingAction {
    Mesh,
    Solve,
    Cancel,
    SampleHole,
    SampleHeat,
}

pub(crate) struct PdeModeler {
    pub model: PdeAppModel,
    pub canvas: canvas::CanvasState,
    pub active_tab: MainTab,
    pub solution_opts: SolutionPlotOptions,
    pub export_names: [String; 2],
    pub formula_edited: bool,
    pub bcs_stale: bool,
    pub bc_notice: Option<String>,
    pub decompose_runner: Runner,
    decompose_delivered: Option<u64>,
    decompose_payload: Arc<Mutex<Option<PreparedFigure>>>,
    pub decompose_error: Option<String>,
    decompose_seq: u64,
    pub plot_runner: Runner,
    plot_delivered: Option<u64>,
    plot_payload: Arc<Mutex<Option<PreparedFigure>>>,
    plot_seq: u64,
    run_seq: u64,
    run_kind: Option<RunKind>,
    /// `run_guard(kind)` when the current Mesh/Solve was submitted. A result
    /// whose guard no longer matches belongs to an older model and is dropped.
    run_guard: Option<u64>,
    run_payload: Arc<Mutex<Option<RunPayload>>>,
    pub geometry_view: Option<PreparedFigure>,
    pub mesh_view: Option<PreparedFigure>,
    pub solution_view: Option<PreparedFigure>,
    pub mesh_nodes: Option<Array>,
    pub mesh_elems: Option<Array>,
    mesh_signature: Option<String>,
    shape_name_drafts: BTreeMap<usize, String>,
    pub shape_name_error: Option<String>,
    /// Refusal for a panel edit that would make a shape degenerate. Same
    /// wording as the canvas info line, shown under the shape list.
    pub shape_geom_error: Option<String>,
    /// Live set-formula error currently drawn under the field.
    pub formula_error_shown: Option<String>,
    shape_num_drafts: BTreeMap<String, String>,
    /// Numeric field that has keyboard focus. Its draft is kept while the
    /// user types; every other draft is resynced from the model.
    num_focus: Option<String>,
    /// `geometry_script()` captured when the current Decompose was submitted.
    decompose_submitted_script: Option<String>,
    pub time_t0_draft: String,
    pub time_t1_draft: String,
    pub time_steps_draft: String,
    /// Refusal for the current t0/t1/Steps text, shown under the fields.
    pub time_field_error: Option<String>,
    pub nodal_solution: Option<Array>,
    pub eigenvalues: Option<Vec<f64>>,
    pub eigen_index: usize,
    pub tlist: Option<Vec<f64>>,
    pub time_index: usize,
    pub playing: bool,
    last_play_tick: Option<Instant>,
    pub node_count: Option<usize>,
    pub element_count: Option<usize>,
    pub min_u: Option<f64>,
    pub max_u: Option<f64>,
    pub time_steps: Option<usize>,
    pub solve_count: usize,
    pub mesh_count: usize,
    pub info_message: Option<String>,
    pub info_is_error: bool,
    scroll_to_edge: Option<usize>,
    last_shape_count: usize,
    pending_action: Option<PendingAction>,
    last_model_fingerprint: Option<u64>,
    export_notice_stale: bool,
    #[cfg(test)]
    pub last_scrolled_edge: Option<usize>,
    #[cfg(test)]
    pub test_solve_delay_ms: u64,
    #[cfg(test)]
    pub test_mesh_delay_ms: u64,
    #[cfg(test)]
    pub controls: BTreeMap<String, egui::Rect>,
    #[cfg(test)]
    pub last_info_color: Option<egui::Color32>,
}

impl Default for PdeModeler {
    fn default() -> Self {
        let model = PdeAppModel::default();
        let time_t0_draft = fmt_field(model.time_range.t0);
        let time_t1_draft = fmt_field(model.time_range.t1);
        let time_steps_draft = model.time_range.steps.to_string();
        Self {
            model,
            canvas: canvas::CanvasState::default(),
            active_tab: MainTab::Draw,
            solution_opts: SolutionPlotOptions::default(),
            export_names: ["model".into(), "results".into()],
            formula_edited: false,
            bcs_stale: false,
            bc_notice: None,
            decompose_runner: Runner::default(),
            decompose_delivered: None,
            decompose_payload: Arc::new(Mutex::new(None)),
            decompose_error: None,
            decompose_seq: 0,
            plot_runner: Runner::default(),
            plot_delivered: None,
            plot_payload: Arc::new(Mutex::new(None)),
            plot_seq: 0,
            run_seq: 0,
            run_kind: None,
            run_guard: None,
            run_payload: Arc::new(Mutex::new(None)),
            geometry_view: None,
            mesh_view: None,
            solution_view: None,
            mesh_nodes: None,
            mesh_elems: None,
            mesh_signature: None,
            shape_name_drafts: BTreeMap::new(),
            shape_name_error: None,
            shape_geom_error: None,
            formula_error_shown: None,
            shape_num_drafts: BTreeMap::new(),
            num_focus: None,
            decompose_submitted_script: None,
            time_t0_draft,
            time_t1_draft,
            time_steps_draft,
            time_field_error: None,
            nodal_solution: None,
            eigenvalues: None,
            eigen_index: 0,
            tlist: None,
            time_index: 0,
            playing: false,
            last_play_tick: None,
            node_count: None,
            element_count: None,
            min_u: None,
            max_u: None,
            time_steps: None,
            solve_count: 0,
            mesh_count: 0,
            info_message: None,
            info_is_error: false,
            scroll_to_edge: None,
            last_shape_count: 0,
            pending_action: None,
            last_model_fingerprint: None,
            export_notice_stale: false,
            #[cfg(test)]
            last_scrolled_edge: None,
            #[cfg(test)]
            test_solve_delay_ms: 0,
            #[cfg(test)]
            test_mesh_delay_ms: 0,
            #[cfg(test)]
            controls: BTreeMap::new(),
            #[cfg(test)]
            last_info_color: None,
        }
    }
}

impl PdeModeler {
    #[cfg(test)]
    fn note(&mut self, name: &str, response: &egui::Response) {
        self.controls.insert(name.to_owned(), response.rect);
    }

    #[cfg(not(test))]
    fn note(&mut self, _name: &str, _response: &egui::Response) {}

    fn clear_solution(&mut self) {
        self.plot_runner.stop();
        self.plot_delivered = None;
        self.nodal_solution = None;
        self.eigenvalues = None;
        self.eigen_index = 0;
        self.tlist = None;
        self.time_index = 0;
        self.playing = false;
        self.last_play_tick = None;
        self.solution_view = None;
        self.min_u = None;
        self.max_u = None;
        self.time_steps = None;
    }

    fn clear_mesh_and_solution(&mut self) {
        self.clear_solution();
        self.mesh_view = None;
        self.mesh_nodes = None;
        self.mesh_elems = None;
        self.mesh_signature = None;
        self.node_count = None;
        self.element_count = None;
    }

    fn clear_all_views(&mut self) {
        self.clear_mesh_and_solution();
        self.geometry_view = None;
    }

    fn set_error(&mut self, msg: impl Into<String>) {
        self.info_message = Some(views::first_line(&msg.into()));
        self.info_is_error = true;
    }

    fn set_info(&mut self, msg: impl Into<String>) {
        self.info_message = Some(views::first_line(&msg.into()));
        self.info_is_error = false;
    }

    fn clear_stale_error(&mut self) {
        if self.info_is_error {
            self.info_message = None;
            self.info_is_error = false;
        }
        self.decompose_error = None;
        self.export_notice_stale = true;
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.run_kind.is_some() || self.decompose_runner.running() || self.plot_runner.running()
    }

    fn model_fingerprint(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.model.shapes.len().hash(&mut h);
        for shape in &self.model.shapes {
            match shape {
                Shape::Rectangle {
                    name,
                    x0,
                    y0,
                    w,
                    h: rh,
                } => {
                    0u8.hash(&mut h);
                    name.hash(&mut h);
                    x0.to_bits().hash(&mut h);
                    y0.to_bits().hash(&mut h);
                    w.to_bits().hash(&mut h);
                    rh.to_bits().hash(&mut h);
                }
                Shape::Circle { name, cx, cy, r } => {
                    1u8.hash(&mut h);
                    name.hash(&mut h);
                    cx.to_bits().hash(&mut h);
                    cy.to_bits().hash(&mut h);
                    r.to_bits().hash(&mut h);
                }
                Shape::Ellipse {
                    name,
                    cx,
                    cy,
                    a,
                    b,
                    angle,
                } => {
                    2u8.hash(&mut h);
                    name.hash(&mut h);
                    cx.to_bits().hash(&mut h);
                    cy.to_bits().hash(&mut h);
                    a.to_bits().hash(&mut h);
                    b.to_bits().hash(&mut h);
                    angle.to_bits().hash(&mut h);
                }
                Shape::Polygon { name, pts } => {
                    3u8.hash(&mut h);
                    name.hash(&mut h);
                    pts.len().hash(&mut h);
                    for p in pts {
                        p[0].to_bits().hash(&mut h);
                        p[1].to_bits().hash(&mut h);
                    }
                }
            }
        }
        self.model.set_formula.hash(&mut h);
        self.model.edge_ids.hash(&mut h);
        self.model.boundary_conditions.len().hash(&mut h);
        for (id, bc) in &self.model.boundary_conditions {
            id.hash(&mut h);
            match bc {
                BoundaryCondition::Dirichlet { u } => {
                    0u8.hash(&mut h);
                    u.hash(&mut h);
                }
                BoundaryCondition::Neumann { g, q } => {
                    1u8.hash(&mut h);
                    g.hash(&mut h);
                    q.hash(&mut h);
                }
            }
        }
        std::mem::discriminant(&self.model.pde_type).hash(&mut h);
        self.model.coeffs.m.hash(&mut h);
        self.model.coeffs.d.hash(&mut h);
        self.model.coeffs.c.hash(&mut h);
        self.model.coeffs.a.hash(&mut h);
        self.model.coeffs.f.hash(&mut h);
        self.model.u0.hash(&mut h);
        self.model.ut0.hash(&mut h);
        self.model.time_range.t0.to_bits().hash(&mut h);
        self.model.time_range.t1.to_bits().hash(&mut h);
        self.model.time_range.steps.hash(&mut h);
        self.model.eigen_range.hash(&mut h);
        self.model.mesh.hmax.hash(&mut h);
        std::mem::discriminant(&self.model.mesh.order).hash(&mut h);
        h.finish()
    }

    /// Current text and error flag (`true` = error, `false` = notice/status)
    /// for the 1-line info bar at the bottom of the Main view.
    pub(crate) fn info_line_state(&self) -> (String, bool) {
        if let Some(msg) = &self.info_message {
            (msg.clone(), self.info_is_error)
        } else if self.is_busy() {
            (String::new(), false)
        } else if self.model.shapes.is_empty() {
            (
                "No shape drawn. Add a shape or load a sample.".to_owned(),
                false,
            )
        } else if self.bcs_stale {
            (
                "Boundary conditions are out of date; press Decompose".to_owned(),
                false,
            )
        } else if let Err(err) = self.validate_export_names() {
            (err, true)
        } else if let (Some(nodes), Some(elems)) = (self.node_count, self.element_count) {
            (format!("{nodes} nodes, {elems} triangles"), false)
        } else {
            ("Ready".to_owned(), false)
        }
    }

    fn mesh_signature_now(&self) -> Option<String> {
        let geo = self.model.geometry_script().ok()?;
        Some(format!(
            "{geo}|{}|{}",
            self.model.mesh.hmax.trim(),
            self.model.mesh.order.as_str()
        ))
    }

    /// What a Mesh or Solve result depends on: the mesh inputs for Mesh,
    /// the whole model for Solve.
    fn run_guard(&self, kind: RunKind) -> u64 {
        match kind {
            RunKind::Mesh => {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                self.mesh_signature_now().hash(&mut h);
                h.finish()
            }
            RunKind::Solve => self.model_fingerprint(),
        }
    }

    fn mesh_is_current(&self) -> bool {
        self.mesh_signature.is_some() && self.mesh_signature == self.mesh_signature_now()
    }

    pub fn validate_export_names(&self) -> Result<(), String> {
        let [m, r] = &self.export_names;
        for name in [m, r] {
            if !PdeAppModel::is_valid_identifier(name) {
                return Err(format!("Invalid export variable name '{name}'."));
            }
            super::check_export_name(name)?;
            if name.starts_with("mrpde_") || name.starts_with("pdem_") {
                return Err(format!("{name} is reserved by PDE Modeler."));
            }
        }
        if m == r {
            return Err(format!(
                "Export variable names must be distinct (both '{m}')."
            ));
        }
        Ok(())
    }

    fn sync_drafts_from_model(&mut self) {
        self.shape_name_drafts.clear();
        self.shape_name_error = None;
        self.shape_geom_error = None;
        self.shape_num_drafts.clear();
        self.num_focus = None;
        self.time_t0_draft = fmt_field(self.model.time_range.t0);
        self.time_t1_draft = fmt_field(self.model.time_range.t1);
        self.time_steps_draft = self.model.time_range.steps.to_string();
        self.time_field_error = None;
    }

    /// Parse the t0 / t1 / Steps text exactly as shown in the panel.
    fn parse_time_drafts(&self) -> Result<(f64, f64, usize), String> {
        let finite = |label: &str, text: &str| -> Result<f64, String> {
            let t = text.trim();
            match t.parse::<f64>() {
                Ok(v) if v.is_finite() => Ok(v),
                _ => Err(format!("Invalid {label} '{t}': must be a finite number.")),
            }
        };
        let t0 = finite("t0", &self.time_t0_draft)?;
        let t1 = finite("t1", &self.time_t1_draft)?;
        let steps_text = self.time_steps_draft.trim();
        let steps = match steps_text.parse::<usize>() {
            Ok(n) if n >= 2 => n,
            _ => {
                return Err(format!(
                    "Invalid Steps '{steps_text}': must be a whole number of at least 2."
                ));
            }
        };
        if t1 <= t0 {
            return Err(format!(
                "Invalid time range: t1 ({}) must exceed t0 ({}).",
                fmt_field(t1),
                fmt_field(t0)
            ));
        }
        Ok((t0, t1, steps))
    }

    /// Time-field text that Solve/Export cannot use, for time-dependent PDEs.
    fn time_drafts_refusal(&self) -> Option<String> {
        if !matches!(
            self.model.pde_type,
            PdeType::Parabolic | PdeType::Hyperbolic
        ) {
            return None;
        }
        self.parse_time_drafts().err()
    }

    fn snap_time_drafts(&mut self) {
        self.time_t0_draft = fmt_field(self.model.time_range.t0);
        self.time_t1_draft = fmt_field(self.model.time_range.t1);
        self.time_steps_draft = self.model.time_range.steps.to_string();
        self.time_field_error = None;
    }

    pub fn load_plate_with_hole(&mut self) {
        self.stop_decompose();
        self.model = PdeAppModel::plate_with_hole();
        self.formula_edited = true;
        self.bcs_stale = false;
        self.bc_notice = None;
        self.decompose_error = None;
        self.info_message = None;
        self.info_is_error = false;
        self.export_notice_stale = true;
        self.canvas.cancel_drafts();
        self.canvas.decomposed_edges.clear();
        self.canvas.selected_shape = None;
        self.canvas.selected_edge = None;
        self.canvas.view.fit_shapes(&self.model.shapes);
        self.last_shape_count = self.model.shapes.len();
        self.sync_drafts_from_model();
        self.clear_all_views();
    }

    pub fn load_plate_heat(&mut self) {
        self.stop_decompose();
        self.model = PdeAppModel::plate_heat();
        self.formula_edited = true;
        self.bcs_stale = false;
        self.bc_notice = None;
        self.decompose_error = None;
        self.info_message = None;
        self.info_is_error = false;
        self.export_notice_stale = true;
        self.canvas.cancel_drafts();
        self.canvas.decomposed_edges.clear();
        self.canvas.selected_shape = None;
        self.canvas.selected_edge = None;
        self.canvas.view.fit_shapes(&self.model.shapes);
        self.last_shape_count = self.model.shapes.len();
        self.sync_drafts_from_model();
        self.clear_all_views();
    }

    /// Notify the modeler that shapes or set formula changed.
    pub fn mark_geometry_changed(&mut self, update_auto_formula: bool) {
        if update_auto_formula && !self.formula_edited {
            self.model.set_formula = self.model.default_set_formula();
        }
        self.clear_all_views();
        self.resync_num_drafts();
        self.shape_name_drafts.clear();
        self.shape_name_error = None;
        self.decompose_error = None;
        self.info_message = None;
        self.info_is_error = false;
        self.export_notice_stale = true;
        if self.model.shapes.len() > self.last_shape_count {
            self.canvas.view.refit_if_outside(&self.model.shapes);
        }
        self.last_shape_count = self.model.shapes.len();
        if self.model.shapes.is_empty() {
            let had_bcs = !self.model.boundary_conditions.is_empty();
            self.model.edge_ids.clear();
            self.model.boundary_conditions.clear();
            self.canvas.decomposed_edges.clear();
            self.canvas.selected_edge = None;
            self.bcs_stale = had_bcs;
            self.bc_notice = had_bcs
                .then(|| "Cleared boundary conditions after removing all shapes.".to_owned());
        } else if !self.model.edge_ids.is_empty()
            || !self.model.boundary_conditions.is_empty()
            || !self.canvas.decomposed_edges.is_empty()
        {
            self.bcs_stale = true;
            self.bc_notice = Some("Geometry changed — click Decompose to update edges.".to_owned());
        }
    }

    /// Drop numeric drafts that are not being edited so the panel shows the
    /// model value. The focused field keeps its text through `1.` / `-0.`.
    fn resync_num_drafts(&mut self) {
        let keep = self.num_focus.clone();
        self.shape_num_drafts
            .retain(|key, _| keep.as_deref() == Some(key.as_str()));
    }

    /// Forget an in-flight Decompose so a late result cannot overwrite the
    /// model that was just loaded.
    fn stop_decompose(&mut self) {
        self.decompose_runner.stop();
        self.decompose_delivered = None;
        self.decompose_submitted_script = None;
    }

    fn decompose_script_matches(&self) -> bool {
        let Some(submitted) = &self.decompose_submitted_script else {
            return false;
        };
        self.model.geometry_script().ok().as_ref() == Some(submitted)
    }

    /// Run `geometry_script()` + `pdegplot(model, 'EdgeLabels', 'on', 'FaceLabels', 'on')`
    /// on the background runner to obtain decomposed edges `E1..En` and the Geometry figure.
    pub fn trigger_decompose(&mut self, ctx: &egui::Context) {
        match self.model.geometry_script() {
            Ok(geo_script) => {
                self.decompose_error = None;
                self.info_message = None;
                self.info_is_error = false;
                self.decompose_submitted_script = Some(geo_script.clone());
                self.decompose_seq += 1;
                let seq = self.decompose_seq;
                let code = format!(
                    "{geo_script}mrpde_dl = decsg(pdem_gd, pdem_sf, pdem_ns);\n\
                     figure(1); pdegplot(model, 'EdgeLabels', 'on', 'FaceLabels', 'on');\n"
                );
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                seq.hash(&mut hasher);
                code.hash(&mut hasher);
                let key = hasher.finish();
                let payload = self.decompose_payload.clone();
                let request = Request {
                    key,
                    inputs: Vec::new(),
                    code,
                    outputs: vec!["mrpde_dl".into()],
                };
                self.decompose_runner.submit_task(
                    ctx,
                    key,
                    Box::new(move |engine| {
                        let outputs = super::run(engine, request)?;
                        let cancel = engine.cancellation();
                        let fig = engine.figures().next().map(|(_, f)| f.clone());
                        let prepared = match fig {
                            Some(f) => Some(views::prepare_figure(f, &cancel, seq)?),
                            None => None,
                        };
                        *payload
                            .lock()
                            .map_err(|_| "Geometry delivery lock failed")? = prepared;
                        Ok(outputs)
                    }),
                );
            }
            Err(err) => {
                let line = views::first_line(&err);
                self.decompose_error = Some(line.clone());
                self.set_error(line);
                self.geometry_view = None;
            }
        }
    }

    fn poll_decompose(&mut self, ctx: &egui::Context) {
        self.decompose_runner.poll(ctx);
        if self.decompose_runner.key() != self.decompose_delivered
            && let Some(result) = self.decompose_runner.result().cloned()
        {
            self.decompose_delivered = self.decompose_runner.key();
            let prepared = self
                .decompose_payload
                .lock()
                .ok()
                .and_then(|mut slot| slot.take());
            // A result belongs to the geometry that was submitted. Editing
            // shapes or the formula, or loading a sample, must not let the
            // old edges overwrite `edge_ids`, prune BCs, or clear stale.
            if !self.decompose_script_matches() {
                return;
            }
            match result {
                Ok(outputs) => match outputs.get("mrpde_dl") {
                    Some(Array::Numeric(m)) => {
                        let edges = canvas::decode_dl_edges(m);
                        // Only outer/hole boundary edges take a condition.
                        let new_ids: Vec<usize> =
                            edges.iter().filter(|e| e.boundary).map(|e| e.id).collect();
                        let before = self.model.boundary_conditions.len();
                        self.model
                            .boundary_conditions
                            .retain(|id, _| new_ids.contains(id));
                        let pruned = before.saturating_sub(self.model.boundary_conditions.len());
                        self.model.edge_ids = new_ids;
                        self.canvas.decomposed_edges = edges;
                        if self
                            .canvas
                            .selected_edge
                            .is_some_and(|id| !self.model.edge_ids.contains(&id))
                        {
                            self.canvas.selected_edge = None;
                        }
                        self.bcs_stale = false;
                        self.bc_notice = (pruned > 0).then(|| {
                            format!("Cleared {pruned} boundary condition(s) for removed edges.")
                        });
                        self.decompose_error = None;
                        self.geometry_view = prepared;
                        self.info_message = None;
                        self.info_is_error = false;
                    }
                    _ => {
                        let msg = "Decompose failed: decsg returned no geometry matrix.";
                        self.decompose_error = Some(msg.into());
                        self.set_error(msg);
                        self.geometry_view = None;
                    }
                },
                Err(err) => {
                    let line = views::first_line(&err);
                    self.decompose_error = Some(line.clone());
                    self.set_error(line);
                    self.geometry_view = None;
                    self.mesh_view = None;
                    self.solution_view = None;
                }
            }
        }
    }

    /// Re-render the Solution view from stored mesh + `NodalSolution`/`Eigenvectors`
    /// column without re-solving (`host.runner` is untouched, `solve_count` unchanged).
    pub fn request_solution_replot(&mut self, ctx: &egui::Context) {
        let (Some(nodes), Some(elems), Some(Array::Numeric(sol))) =
            (&self.mesh_nodes, &self.mesh_elems, &self.nodal_solution)
        else {
            return;
        };
        let col_idx = match self.model.pde_type {
            PdeType::Parabolic | PdeType::Hyperbolic => self.time_index,
            PdeType::Eigen => self.eigen_index,
            PdeType::Elliptic => 0,
        };
        if sol.cols() == 0 || col_idx >= sol.cols() {
            return;
        }
        let n_rows = sol.rows();
        let mut col_data = Vec::with_capacity(n_rows);
        for r in 0..n_rows {
            col_data.push(sol.get(r, col_idx));
        }
        let Ok(col_mat) = Matrix::from_column_vec(n_rows, 1, col_data) else {
            return;
        };
        self.plot_seq += 1;
        let seq = self.plot_seq;
        let fixed_clim = if matches!(
            self.model.pde_type,
            PdeType::Parabolic | PdeType::Hyperbolic
        ) {
            self.nodal_solution
                .as_ref()
                .and_then(views::global_solution_clim)
        } else {
            None
        };
        let mut code = views::pdeplot_call(
            "mrpde_mesh_nodes, mrpde_mesh_elems",
            "mrpde_u_col",
            self.solution_opts,
        );
        if let Some([lo, hi]) = fixed_clim {
            code.push_str(&format!("clim([{lo:?}, {hi:?}]);\n"));
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        seq.hash(&mut hasher);
        col_idx.hash(&mut hasher);
        self.solution_opts.hash(&mut hasher);
        let key = hasher.finish();
        let request = Request {
            key,
            inputs: vec![
                ("mrpde_mesh_nodes".into(), nodes.clone()),
                ("mrpde_mesh_elems".into(), elems.clone()),
                ("mrpde_u_col".into(), Array::Numeric(col_mat)),
            ],
            code,
            outputs: Vec::new(),
        };
        let payload = self.plot_payload.clone();
        self.plot_runner.submit_task(
            ctx,
            key,
            Box::new(move |engine| {
                let outputs = super::run(engine, request)?;
                let cancel = engine.cancellation();
                let fig = engine
                    .figures()
                    .next()
                    .map(|(_, f)| f.clone())
                    .ok_or_else(|| "pdeplot produced no figure".to_owned())?;
                let prepared = views::prepare_figure(fig, &cancel, seq)?;
                *payload.lock().map_err(|_| "Plot delivery lock failed")? = Some(prepared);
                Ok(outputs)
            }),
        );
    }

    fn poll_plot(&mut self, ctx: &egui::Context) {
        self.plot_runner.poll(ctx);
        if self.plot_runner.key() != self.plot_delivered
            && let Some(result) = self.plot_runner.result().cloned()
        {
            self.plot_delivered = self.plot_runner.key();
            let prepared = self
                .plot_payload
                .lock()
                .ok()
                .and_then(|mut slot| slot.take());
            match result {
                Ok(_) => {
                    if let Some(fig) = prepared {
                        self.solution_view = Some(fig);
                    }
                }
                Err(err) => {
                    self.set_error(err);
                }
            }
        }
    }

    /// Move the time slider to `idx` (clamped) and replot from stored NodalSolution
    /// without re-solving.
    pub fn set_time_index(&mut self, idx: usize, ctx: &egui::Context) {
        let count = self.tlist.as_ref().map_or(0, Vec::len);
        if count == 0 {
            return;
        }
        let clamped = idx.min(count - 1);
        if self.time_index != clamped {
            self.time_index = clamped;
            self.request_solution_replot(ctx);
        }
    }

    /// Select eigenmode `idx` (clamped) and replot from stored Eigenvectors
    /// without re-solving.
    pub fn set_eigen_index(&mut self, idx: usize, ctx: &egui::Context) {
        let count = self.eigenvalues.as_ref().map_or(0, Vec::len);
        if count == 0 {
            return;
        }
        let clamped = idx.min(count - 1);
        if self.eigen_index != clamped {
            self.eigen_index = clamped;
            if let Some((lo, hi)) = self
                .nodal_solution
                .as_ref()
                .and_then(|sol| column_range(sol, clamped))
            {
                self.min_u = Some(lo);
                self.max_u = Some(hi);
            }
            self.request_solution_replot(ctx);
        }
    }

    /// Switch PDE equation type and normalize unused coefficients so the
    /// generated `specifyCoefficients` call stays consistent.
    pub fn set_pde_type(&mut self, next: PdeType) {
        if self.model.pde_type == next {
            return;
        }
        self.model.pde_type = next;
        match next {
            PdeType::Elliptic => {
                self.model.coeffs.m = "0".into();
                self.model.coeffs.d = "0".into();
            }
            PdeType::Parabolic => {
                self.model.coeffs.m = "0".into();
                if is_zero_or_empty(&self.model.coeffs.d) {
                    self.model.coeffs.d = "1".into();
                }
            }
            PdeType::Hyperbolic => {
                self.model.coeffs.d = "0".into();
                if is_zero_or_empty(&self.model.coeffs.m) {
                    self.model.coeffs.m = "1".into();
                }
            }
            PdeType::Eigen => {
                self.model.coeffs.m = "0".into();
                self.model.coeffs.f = "0".into();
                if is_zero_or_empty(&self.model.coeffs.d) {
                    self.model.coeffs.d = "1".into();
                }
            }
        }
        self.clear_solution();
        self.clear_stale_error();
    }

    /// Apply the selected edge's BC (or first assigned BC, else Dirichlet u=0)
    /// to all edges in `edge_ids` that currently have no BC assigned.
    pub fn apply_bc_to_all_unassigned(&mut self) {
        let template = self
            .canvas
            .selected_edge
            .and_then(|id| self.model.boundary_conditions.get(&id).cloned())
            .or_else(|| self.model.boundary_conditions.values().next().cloned())
            .unwrap_or_else(|| BoundaryCondition::Dirichlet { u: "0".into() });
        for &id in &self.model.edge_ids {
            self.model
                .boundary_conditions
                .entry(id)
                .or_insert_with(|| template.clone());
        }
        self.clear_solution();
        self.clear_stale_error();
    }

    fn show_geometry_section(&mut self, ui: &mut egui::Ui, p: &tokens::Palette) {
        ui.label(egui::RichText::new("Geometry").strong().color(p.text));
        ui.add_space(tokens::XS);

        // Set formula + Decompose stay above the shape list so they remain
        // visible above the fold even with many shapes.
        ui.label(
            egui::RichText::new("Set formula")
                .size(tokens::BODY)
                .color(p.text),
        );
        let sf_resp = ui.add(tokens::input(
            egui::TextEdit::singleline(&mut self.model.set_formula)
                .desired_width(f32::INFINITY)
                .hint_text("e.g. R1-C1"),
        ));
        self.note("Set formula", &sf_resp);
        if sf_resp.changed() {
            self.formula_edited = true;
            self.mark_geometry_changed(false);
        }
        self.formula_error_shown = None;
        if (!self.model.shapes.is_empty() || !self.model.set_formula.trim().is_empty())
            && let Err(err) = self.model.validate_set_formula()
        {
            let resp = ui.label(egui::RichText::new(&err).size(LABEL_SIZE).color(p.error));
            self.note("Set formula error", &resp);
            self.formula_error_shown = Some(err);
        }

        ui.add_space(tokens::XS);
        let busy = self.decompose_runner.running();
        let decomp_btn = ui.add_enabled(
            !busy && !self.model.shapes.is_empty(),
            egui::Button::new(if busy { "Decomposing…" } else { "Decompose" })
                .min_size(egui::vec2(ui.available_width(), tokens::CONTROL)),
        );
        self.note("Decompose", &decomp_btn);
        if decomp_btn.clicked() {
            self.trigger_decompose(ui.ctx());
        }
        ui.add_space(tokens::S);

        if self.model.shapes.is_empty() {
            self.shape_geom_error = None;
            self.shape_name_error = None;
            tokens::helper(ui, "Draw shapes on the canvas or load a sample.");
        } else {
            let mut delete_idx: Option<usize> = None;
            let mut geom_changed = false;
            let mut rename_committed = false;

            let use_scroll = self.model.shapes.len() > 4;
            let draw_shapes = |ui: &mut egui::Ui| {
                for i in 0..self.model.shapes.len() {
                    let selected = self.canvas.selected_shape == Some(i);
                    let frame = egui::Frame::new()
                        .fill(if selected { p.accent_soft } else { p.surface })
                        .stroke(egui::Stroke::new(
                            1.0,
                            if selected { p.accent } else { p.hairline },
                        ))
                        .corner_radius(tokens::CONTROL_RADIUS)
                        .inner_margin(tokens::XS as i8);

                    frame.show(ui, |ui| {
                        let kind_label = match &self.model.shapes[i] {
                            Shape::Rectangle { .. } => "Rect",
                            Shape::Circle { .. } => "Circle",
                            Shape::Ellipse { .. } => "Ellipse",
                            Shape::Polygon { .. } => "Poly",
                        };

                        // Row 1: name edit + type badge + delete button.
                        ui.horizontal(|ui| {
                            let current_name = self.model.shapes[i].name().to_owned();
                            let (name_resp, changed_candidate) = {
                                let draft = self
                                    .shape_name_drafts
                                    .entry(i)
                                    .or_insert_with(|| current_name.clone());
                                let resp = ui.add(tokens::input(
                                    egui::TextEdit::singleline(draft).desired_width(84.0),
                                ));
                                let cand = resp.changed().then(|| draft.clone());
                                (resp, cand)
                            };
                            self.note(&format!("Shape:{i}:name"), &name_resp);
                            if name_resp.gained_focus() {
                                self.canvas.selected_shape = Some(i);
                            }
                            if let Some(candidate) = changed_candidate {
                                match self.model.rename_shape(i, &candidate) {
                                    Ok(()) => {
                                        self.shape_name_error = None;
                                        let new_name = self.model.shapes[i].name().to_owned();
                                        if new_name != current_name {
                                            self.model.set_formula = replace_formula_token(
                                                &self.model.set_formula,
                                                &current_name,
                                                &new_name,
                                            );
                                            rename_committed = true;
                                        }
                                    }
                                    Err(err) => {
                                        self.shape_name_error = Some(err);
                                    }
                                }
                            }
                            if name_resp.lost_focus() {
                                self.shape_name_drafts.remove(&i);
                                if self.model.validate_shape_names().is_ok() {
                                    self.shape_name_error = None;
                                }
                            }

                            let badge = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(kind_label)
                                        .size(LABEL_SIZE)
                                        .color(p.text),
                                )
                                .sense(egui::Sense::click()),
                            );
                            if badge.clicked() {
                                self.canvas.selected_shape = Some(i);
                            }

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let del_btn = ui.add(
                                        egui::Button::new("×").min_size(egui::vec2(24.0, 24.0)),
                                    );
                                    self.note(&format!("Shape:{i}:delete"), &del_btn);
                                    if del_btn.clicked() {
                                        delete_idx = Some(i);
                                    }
                                },
                            );
                        });

                        // Row 2: numeric fields for this shape.
                        ui.add_space(tokens::XS);
                        match self.model.shapes[i].clone() {
                            Shape::Rectangle { x0, y0, w, h, .. } => {
                                let mut vals = [x0, y0, w, h];
                                let labels = ["x0", "y0", "w", "h"];
                                let mut edited = false;
                                egui::Grid::new(ui.id().with(("rect_fields", i)))
                                    .num_columns(4)
                                    .spacing(egui::vec2(tokens::XS, tokens::XS))
                                    .show(ui, |ui| {
                                        for k in 0..4 {
                                            let key = format!("Shape:{i}:{}", labels[k]);
                                            if self.num_field(
                                                ui,
                                                labels[k],
                                                &key,
                                                &mut vals[k],
                                                k >= 2,
                                            ) {
                                                edited = true;
                                            }
                                            if k == 1 {
                                                ui.end_row();
                                            }
                                        }
                                    });
                                if edited {
                                    if let Shape::Rectangle {
                                        x0: rx,
                                        y0: ry,
                                        w: rw,
                                        h: rh,
                                        ..
                                    } = &mut self.model.shapes[i]
                                    {
                                        *rx = vals[0];
                                        *ry = vals[1];
                                        *rw = vals[2];
                                        *rh = vals[3];
                                    }
                                    geom_changed = true;
                                }
                            }
                            Shape::Circle { cx, cy, r, .. } => {
                                let mut vals = [cx, cy, r];
                                let labels = ["cx", "cy", "r"];
                                let mut edited = false;
                                egui::Grid::new(ui.id().with(("circle_fields", i)))
                                    .num_columns(4)
                                    .spacing(egui::vec2(tokens::XS, tokens::XS))
                                    .show(ui, |ui| {
                                        for k in 0..3 {
                                            let key = format!("Shape:{i}:{}", labels[k]);
                                            if self.num_field(
                                                ui,
                                                labels[k],
                                                &key,
                                                &mut vals[k],
                                                k == 2,
                                            ) {
                                                edited = true;
                                            }
                                            if k == 1 {
                                                ui.end_row();
                                            }
                                        }
                                    });
                                if edited {
                                    if let Shape::Circle {
                                        cx: rcx,
                                        cy: rcy,
                                        r: rr,
                                        ..
                                    } = &mut self.model.shapes[i]
                                    {
                                        *rcx = vals[0];
                                        *rcy = vals[1];
                                        *rr = vals[2];
                                    }
                                    geom_changed = true;
                                }
                            }
                            Shape::Ellipse {
                                cx,
                                cy,
                                a,
                                b,
                                angle,
                                ..
                            } => {
                                let mut vals = [cx, cy, a, b, angle];
                                let labels = ["cx", "cy", "a", "b", "ang"];
                                let mut edited = false;
                                egui::Grid::new(ui.id().with(("ellipse_fields", i)))
                                    .num_columns(4)
                                    .spacing(egui::vec2(tokens::XS, tokens::XS))
                                    .show(ui, |ui| {
                                        for k in 0..5 {
                                            let key = format!("Shape:{i}:{}", labels[k]);
                                            if self.num_field(
                                                ui,
                                                labels[k],
                                                &key,
                                                &mut vals[k],
                                                k == 2 || k == 3,
                                            ) {
                                                edited = true;
                                            }
                                            if k == 1 || k == 3 {
                                                ui.end_row();
                                            }
                                        }
                                    });
                                if edited {
                                    if let Shape::Ellipse {
                                        cx: rcx,
                                        cy: rcy,
                                        a: ra,
                                        b: rb,
                                        angle: rang,
                                        ..
                                    } = &mut self.model.shapes[i]
                                    {
                                        *rcx = vals[0];
                                        *rcy = vals[1];
                                        *ra = vals[2];
                                        *rb = vals[3];
                                        *rang = vals[4];
                                    }
                                    geom_changed = true;
                                }
                            }
                            Shape::Polygon { mut pts, .. } => {
                                let mut edited = false;
                                ui.label(
                                    egui::RichText::new(format!("{} vertices", pts.len()))
                                        .size(LABEL_SIZE)
                                        .color(p.text),
                                );
                                egui::Grid::new(ui.id().with(("poly_fields", i)))
                                    .num_columns(4)
                                    .spacing(egui::vec2(tokens::XS, tokens::XS))
                                    .show(ui, |ui| {
                                        for (v_idx, pt) in pts.iter_mut().enumerate() {
                                            let kx = format!("Shape:{i}:x{v_idx}");
                                            let ky = format!("Shape:{i}:y{v_idx}");
                                            if self.num_field(
                                                ui,
                                                &format!("x{}", v_idx + 1),
                                                &kx,
                                                &mut pt[0],
                                                false,
                                            ) {
                                                edited = true;
                                            }
                                            if self.num_field(
                                                ui,
                                                &format!("y{}", v_idx + 1),
                                                &ky,
                                                &mut pt[1],
                                                false,
                                            ) {
                                                edited = true;
                                            }
                                            ui.end_row();
                                        }
                                    });
                                if edited && canvas::validate_polygon(&pts).is_ok() {
                                    if let Shape::Polygon { pts: target, .. } =
                                        &mut self.model.shapes[i]
                                    {
                                        *target = pts;
                                    }
                                    geom_changed = true;
                                }
                            }
                        }
                    });
                    ui.add_space(tokens::XS);
                }
            };

            let list_rect = if use_scroll {
                egui::ScrollArea::vertical()
                    .id_salt("pde_shapes_scroll")
                    .max_height(240.0)
                    .auto_shrink([false, true])
                    .show_themed(ui, draw_shapes)
                    .inner_rect
            } else {
                ui.scope(draw_shapes).response.rect
            };
            #[cfg(test)]
            self.controls.insert("ShapeList".to_owned(), list_rect);
            let _ = list_rect;

            if let Some(err) = &self.shape_name_error {
                ui.label(egui::RichText::new(err).size(LABEL_SIZE).color(p.error));
            }
            let refusal = self.shape_draft_refusal();
            self.shape_geom_error = refusal.clone();
            if let Some(err) = refusal {
                let resp = ui.label(egui::RichText::new(err).size(LABEL_SIZE).color(p.error));
                self.note("Shape geometry error", &resp);
            }

            if let Some(idx) = delete_idx {
                self.num_focus = None;
                self.shape_num_drafts.clear();
                self.model.shapes.remove(idx);
                if self.canvas.selected_shape == Some(idx) {
                    self.canvas.selected_shape = None;
                } else if let Some(sel) = self.canvas.selected_shape
                    && sel > idx
                {
                    self.canvas.selected_shape = Some(sel - 1);
                }
                self.mark_geometry_changed(true);
            } else if rename_committed {
                self.mark_geometry_changed(true);
            } else if geom_changed {
                self.mark_geometry_changed(false);
            }
        }
    }

    /// Panel drafts that would make a shape degenerate, in the same words as
    /// the canvas info line. Incomplete text (`-`, `1.`) is not a refusal.
    fn shape_draft_refusal(&self) -> Option<String> {
        for (i, shape) in self.model.shapes.iter().enumerate() {
            let refusal = match shape {
                Shape::Rectangle { .. } => (self.draft_is_non_positive(&format!("Shape:{i}:w"))
                    || self.draft_is_non_positive(&format!("Shape:{i}:h")))
                .then(|| {
                    "Refused degenerate rectangle: width and height must be positive.".to_owned()
                }),
                Shape::Circle { .. } => self
                    .draft_is_non_positive(&format!("Shape:{i}:r"))
                    .then(|| "Refused degenerate circle: radius must be positive.".to_owned()),
                Shape::Ellipse { .. } => (self.draft_is_non_positive(&format!("Shape:{i}:a"))
                    || self.draft_is_non_positive(&format!("Shape:{i}:b")))
                .then(|| "Refused degenerate ellipse: semi-axes must be positive.".to_owned()),
                Shape::Polygon { pts, .. } => self.polygon_draft_refusal(i, pts),
            };
            if refusal.is_some() {
                return refusal;
            }
        }
        None
    }

    fn draft_is_non_positive(&self, key: &str) -> bool {
        self.shape_num_drafts
            .get(key)
            .and_then(|text| text.trim().parse::<f64>().ok())
            .is_some_and(|v| v.is_finite() && v <= 0.0)
    }

    fn polygon_draft_refusal(&self, index: usize, pts: &[[f64; 2]]) -> Option<String> {
        let mut shown = pts.to_vec();
        let mut changed = false;
        for (v, pt) in shown.iter_mut().enumerate() {
            for (axis, coord) in [('x', 0usize), ('y', 1usize)] {
                let key = format!("Shape:{index}:{axis}{v}");
                let Some(draft) = self.shape_num_drafts.get(&key) else {
                    continue;
                };
                match draft.trim().parse::<f64>() {
                    Ok(val) if val.is_finite() => {
                        if (val - pt[coord]).abs() > 1e-12 {
                            changed = true;
                            pt[coord] = val;
                        }
                    }
                    _ => return None,
                }
            }
        }
        if !changed {
            return None;
        }
        canvas::validate_polygon(&shown).err()
    }

    fn num_field(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        key: &str,
        value: &mut f64,
        positive_only: bool,
    ) -> bool {
        let p = tokens::palette(ui.ctx());
        ui.label(egui::RichText::new(label).size(LABEL_SIZE).color(p.text));
        let full_value = fmt_field(*value);
        let (resp, parsed_val) = {
            let draft = self
                .shape_num_drafts
                .entry(key.to_owned())
                .or_insert_with(|| full_value.clone());
            let resp = ui
                .add(
                    tokens::input(egui::TextEdit::singleline(draft).desired_width(72.0))
                        .min_size(egui::vec2(72.0, tokens::CONTROL)),
                )
                .on_hover_text(full_value);
            let parsed = resp
                .changed()
                .then(|| draft.trim().parse::<f64>().ok())
                .flatten();
            (resp, parsed)
        };
        self.note(key, &resp);
        if resp.has_focus() {
            self.num_focus = Some(key.to_owned());
        }
        let applied = if let Some(parsed) = parsed_val
            && parsed.is_finite()
            && (!positive_only || parsed > 0.0)
        {
            *value = parsed;
            true
        } else {
            false
        };
        // Focus loss snaps the field back to the model value. Until then the
        // typed text (including a trailing '.' or a leading '-') stays put.
        if resp.lost_focus() {
            self.shape_num_drafts.remove(key);
            if self.num_focus.as_deref() == Some(key) {
                self.num_focus = None;
            }
        }
        applied
    }

    fn show_bc_section(&mut self, ui: &mut egui::Ui, p: &tokens::Palette) {
        ui.add_space(tokens::M);
        ui.label(
            egui::RichText::new("Boundary conditions")
                .strong()
                .color(p.text),
        );
        ui.add_space(tokens::XS);

        if let Some(notice) = &self.bc_notice {
            ui.label(
                egui::RichText::new(notice)
                    .size(LABEL_SIZE)
                    .color(p.text_muted),
            );
        }

        if self.model.edge_ids.is_empty() {
            tokens::helper(ui, "Click Decompose to list geometry edges.");
            return;
        }

        ui.add_space(tokens::XS);
        let apply_btn = ui.add(
            egui::Button::new("Apply to all unassigned")
                .min_size(egui::vec2(ui.available_width(), tokens::CONTROL)),
        );
        self.note("Apply to all unassigned", &apply_btn);
        if apply_btn.clicked() {
            self.apply_bc_to_all_unassigned();
        }
        ui.add_space(tokens::XS);

        let edge_ids = self.model.edge_ids.clone();
        let mut bc_changed = false;
        for edge_id in edge_ids {
            let selected = self.canvas.selected_edge == Some(edge_id);
            let frame = egui::Frame::new()
                .fill(if selected { p.accent_soft } else { p.surface })
                .stroke(egui::Stroke::new(
                    1.0,
                    if selected { p.accent } else { p.hairline },
                ))
                .corner_radius(tokens::CONTROL_RADIUS)
                .inner_margin(tokens::XS as i8);

            let inner = frame.show(ui, |ui| {
                let current = self.model.boundary_conditions.get(&edge_id).cloned();
                let is_dir = matches!(current, Some(BoundaryCondition::Dirichlet { .. }));
                let is_neu = matches!(current, Some(BoundaryCondition::Neumann { .. }));

                ui.horizontal(|ui| {
                    let lbl = ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!("E{edge_id}"))
                                .strong()
                                .color(if selected { p.accent_text } else { p.text }),
                        )
                        .sense(egui::Sense::click()),
                    );
                    self.note(&format!("Edge:{edge_id}:row"), &lbl);
                    if lbl.clicked() {
                        self.canvas.selected_edge = Some(edge_id);
                    }

                    let dir_btn = tokens::toggle(ui, is_dir, "Dirichlet");
                    self.note(&format!("Edge:{edge_id}:Dirichlet"), &dir_btn);
                    if dir_btn.clicked() {
                        self.canvas.selected_edge = Some(edge_id);
                        if !is_dir {
                            self.model
                                .boundary_conditions
                                .insert(edge_id, BoundaryCondition::Dirichlet { u: "0".into() });
                            bc_changed = true;
                        }
                    }

                    let neu_btn = tokens::toggle(ui, is_neu, "Neumann");
                    self.note(&format!("Edge:{edge_id}:Neumann"), &neu_btn);
                    if neu_btn.clicked() {
                        self.canvas.selected_edge = Some(edge_id);
                        if !is_neu {
                            self.model.boundary_conditions.insert(
                                edge_id,
                                BoundaryCondition::Neumann {
                                    g: "0".into(),
                                    q: "0".into(),
                                },
                            );
                            bc_changed = true;
                        }
                    }
                });

                let mut noted_fields: Vec<(String, egui::Response)> = Vec::new();
                if let Some(bc) = self.model.boundary_conditions.get_mut(&edge_id) {
                    ui.add_space(tokens::XS);
                    match bc {
                        BoundaryCondition::Dirichlet { u } => {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("u =").color(p.text));
                                let r = ui.add(
                                    tokens::input(
                                        egui::TextEdit::singleline(u).desired_width(120.0),
                                    )
                                    .min_size(egui::vec2(80.0, tokens::CONTROL)),
                                );
                                if r.changed() {
                                    bc_changed = true;
                                }
                                noted_fields.push((format!("Edge:{edge_id}:u"), r));
                            });
                        }
                        BoundaryCondition::Neumann { g, q } => {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("g =").color(p.text));
                                let rg = ui.add(
                                    tokens::input(
                                        egui::TextEdit::singleline(g).desired_width(64.0),
                                    )
                                    .min_size(egui::vec2(52.0, tokens::CONTROL)),
                                );
                                ui.label(egui::RichText::new("q =").color(p.text));
                                let rq = ui.add(
                                    tokens::input(
                                        egui::TextEdit::singleline(q).desired_width(64.0),
                                    )
                                    .min_size(egui::vec2(52.0, tokens::CONTROL)),
                                );
                                if rg.changed() || rq.changed() {
                                    bc_changed = true;
                                }
                                noted_fields.push((format!("Edge:{edge_id}:g"), rg));
                                noted_fields.push((format!("Edge:{edge_id}:q"), rq));
                            });
                        }
                    }
                } else {
                    ui.label(
                        egui::RichText::new("Unassigned")
                            .size(LABEL_SIZE)
                            .color(p.text),
                    );
                }
                for (name, resp) in &noted_fields {
                    self.note(name, resp);
                }
                if let Some(Err(err)) = self
                    .model
                    .boundary_conditions
                    .get(&edge_id)
                    .map(|bc| bc.validate_expressions(edge_id))
                {
                    ui.label(egui::RichText::new(err).size(LABEL_SIZE).color(p.error));
                }
            });
            if self.scroll_to_edge == Some(edge_id) {
                inner.response.scroll_to_me(Some(egui::Align::Center));
                self.scroll_to_edge = None;
                #[cfg(test)]
                {
                    self.last_scrolled_edge = Some(edge_id);
                }
            }
            ui.add_space(tokens::XS);
        }
        if bc_changed {
            self.clear_solution();
            self.clear_stale_error();
        }
    }

    fn show_pde_section(&mut self, ui: &mut egui::Ui, p: &tokens::Palette) {
        ui.add_space(tokens::M);
        ui.label(egui::RichText::new("PDE").strong().color(p.text));
        ui.add_space(tokens::XS);

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = tokens::XS;
            for (kind, label) in [
                (PdeType::Elliptic, "Elliptic"),
                (PdeType::Parabolic, "Parabolic"),
                (PdeType::Hyperbolic, "Hyperbolic"),
                (PdeType::Eigen, "Eigen"),
            ] {
                let resp = tokens::toggle(ui, self.model.pde_type == kind, label);
                self.note(&format!("PDE:{label}"), &resp);
                if resp.clicked() {
                    self.set_pde_type(kind);
                }
            }
        });

        ui.add_space(tokens::XS);
        let eq_line = match self.model.pde_type {
            PdeType::Elliptic => "-div(c grad u) + a u = f",
            PdeType::Parabolic => "d du/dt - div(c grad u) + a u = f",
            PdeType::Hyperbolic => "m d²u/dt² - div(c grad u) + a u = f",
            PdeType::Eigen => "-div(c grad u) + a u = λ d u",
        };
        ui.label(egui::RichText::new(eq_line).size(LABEL_SIZE).color(p.text));
        ui.add_space(tokens::XS);

        let mut changed = false;
        egui::Grid::new(ui.id().with("pde_coeffs"))
            .num_columns(2)
            .spacing(egui::vec2(tokens::S, tokens::XS))
            .show(ui, |ui| {
                let mut coeff_row =
                    |ui: &mut egui::Ui, label: &str, key: &str, val: &mut String| {
                        ui.label(egui::RichText::new(label).color(p.text));
                        let r = ui.add(tokens::input(
                            egui::TextEdit::singleline(val).desired_width(140.0),
                        ));
                        #[cfg(test)]
                        self.controls.insert(key.to_owned(), r.rect);
                        let _ = key;
                        if r.changed() {
                            changed = true;
                        }
                        ui.end_row();
                    };

                match self.model.pde_type {
                    PdeType::Elliptic => {
                        coeff_row(ui, "c", "PDE:c", &mut self.model.coeffs.c);
                        coeff_row(ui, "a", "PDE:a", &mut self.model.coeffs.a);
                        coeff_row(ui, "f", "PDE:f", &mut self.model.coeffs.f);
                    }
                    PdeType::Parabolic => {
                        coeff_row(ui, "d", "PDE:d", &mut self.model.coeffs.d);
                        coeff_row(ui, "c", "PDE:c", &mut self.model.coeffs.c);
                        coeff_row(ui, "a", "PDE:a", &mut self.model.coeffs.a);
                        coeff_row(ui, "f", "PDE:f", &mut self.model.coeffs.f);
                        coeff_row(ui, "u0", "PDE:u0", &mut self.model.u0);
                    }
                    PdeType::Hyperbolic => {
                        coeff_row(ui, "m", "PDE:m", &mut self.model.coeffs.m);
                        coeff_row(ui, "c", "PDE:c", &mut self.model.coeffs.c);
                        coeff_row(ui, "a", "PDE:a", &mut self.model.coeffs.a);
                        coeff_row(ui, "f", "PDE:f", &mut self.model.coeffs.f);
                        coeff_row(ui, "u0", "PDE:u0", &mut self.model.u0);
                        coeff_row(ui, "ut0", "PDE:ut0", &mut self.model.ut0);
                    }
                    PdeType::Eigen => {
                        coeff_row(ui, "d", "PDE:d", &mut self.model.coeffs.d);
                        coeff_row(ui, "c", "PDE:c", &mut self.model.coeffs.c);
                        coeff_row(ui, "a", "PDE:a", &mut self.model.coeffs.a);
                        coeff_row(ui, "Range", "PDE:range", &mut self.model.eigen_range);
                    }
                }
            });

        if let Err(err) = self.model.validate_coefficient_expressions() {
            let resp = ui.label(egui::RichText::new(err).size(LABEL_SIZE).color(p.error));
            self.note("PDE:expression_error", &resp);
        }

        if matches!(
            self.model.pde_type,
            PdeType::Parabolic | PdeType::Hyperbolic
        ) {
            ui.add_space(tokens::XS);
            let mut time_responses: Option<[egui::Response; 3]> = None;
            egui::Grid::new(ui.id().with("pde_time_range"))
                .num_columns(2)
                .spacing(egui::vec2(tokens::S, tokens::XS))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("t0").color(p.text));
                    let r0 = ui.add(tokens::input(
                        egui::TextEdit::singleline(&mut self.time_t0_draft).desired_width(140.0),
                    ));
                    self.note("PDE:t0", &r0);
                    ui.end_row();

                    ui.label(egui::RichText::new("t1").color(p.text));
                    let r1 = ui.add(tokens::input(
                        egui::TextEdit::singleline(&mut self.time_t1_draft).desired_width(140.0),
                    ));
                    self.note("PDE:t1", &r1);
                    ui.end_row();

                    ui.label(egui::RichText::new("Steps").color(p.text));
                    let rs = ui.add(tokens::input(
                        egui::TextEdit::singleline(&mut self.time_steps_draft).desired_width(140.0),
                    ));
                    self.note("PDE:steps", &rs);
                    ui.end_row();
                    time_responses = Some([r0, r1, rs]);
                });
            if let Some(responses) = time_responses {
                if responses.iter().any(egui::Response::changed) {
                    match self.parse_time_drafts() {
                        Ok((t0, t1, steps)) => {
                            self.time_field_error = None;
                            let tr = &mut self.model.time_range;
                            if tr.t0 != t0 || tr.t1 != t1 || tr.steps != steps {
                                *tr = matrust::engine::pde_app_model::TimeRange { t0, t1, steps };
                                changed = true;
                            }
                        }
                        Err(err) => {
                            self.time_field_error = Some(err.clone());
                            self.set_error(err);
                        }
                    }
                }
                // Like the shape fields: leaving a field with refused text
                // snaps all three back to the model (the info line keeps
                // why). Not being edited, they always show the model values.
                if !responses.iter().any(egui::Response::has_focus)
                    || (self.time_field_error.is_some()
                        && responses.iter().any(egui::Response::lost_focus))
                {
                    self.snap_time_drafts();
                }
            }
            if let Some(err) = &self.time_field_error {
                ui.label(egui::RichText::new(err).size(LABEL_SIZE).color(p.error));
            }
        }

        if changed {
            self.clear_solution();
            self.clear_stale_error();
        }
    }

    fn show_mesh_section(&mut self, ui: &mut egui::Ui, p: &tokens::Palette) {
        ui.add_space(tokens::M);
        ui.label(egui::RichText::new("Mesh").strong().color(p.text));
        ui.add_space(tokens::XS);

        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Hmax").color(p.text));
            let r = ui.add(tokens::input(
                egui::TextEdit::singleline(&mut self.model.mesh.hmax)
                    .desired_width(120.0)
                    .hint_text("auto"),
            ));
            self.note("Mesh:Hmax", &r);
            if r.changed() {
                changed = true;
            }
        });
        ui.add_space(tokens::XS);
        ui.horizontal(|ui| {
            let lin = tokens::toggle(
                ui,
                self.model.mesh.order == GeometricOrder::Linear,
                "Linear",
            );
            self.note("Mesh:Linear", &lin);
            if lin.clicked() && self.model.mesh.order != GeometricOrder::Linear {
                self.model.mesh.order = GeometricOrder::Linear;
                changed = true;
            }
            let quad = tokens::toggle(
                ui,
                self.model.mesh.order == GeometricOrder::Quadratic,
                "Quadratic",
            );
            self.note("Mesh:Quadratic", &quad);
            if quad.clicked() && self.model.mesh.order != GeometricOrder::Quadratic {
                self.model.mesh.order = GeometricOrder::Quadratic;
                changed = true;
            }
        });
        if changed {
            self.clear_mesh_and_solution();
            self.clear_stale_error();
        }
    }

    fn show_main_area(&mut self, ui: &mut egui::Ui, runner_busy: bool) {
        let p = tokens::palette(ui.ctx());
        let any_busy = runner_busy || self.decompose_runner.running() || self.plot_runner.running();

        // Top row: view tabs + Mesh / Solve / Cancel actions.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = tokens::XS;
            for tab in MainTab::ALL {
                let resp = tokens::toggle(ui, self.active_tab == tab, tab.label());
                self.note(&format!("Tab:{}", tab.label()), &resp);
                if resp.clicked() && self.active_tab != tab {
                    self.active_tab = tab;
                    if tab == MainTab::Geometry
                        && self.geometry_view.is_none()
                        && !self.decompose_runner.running()
                        && !self.model.shapes.is_empty()
                    {
                        self.trigger_decompose(ui.ctx());
                    }
                }
            }

            ui.add_space(tokens::S);
            let mesh_btn =
                ui.add(egui::Button::new("Mesh").min_size(egui::vec2(0.0, tokens::CONTROL)));
            self.note("Mesh", &mesh_btn);
            if mesh_btn.clicked() {
                self.pending_action = Some(PendingAction::Mesh);
            }

            let solve_btn = ui.add(
                egui::Button::new(egui::RichText::new("Solve").strong())
                    .min_size(egui::vec2(0.0, tokens::CONTROL)),
            );
            self.note("Solve", &solve_btn);
            if solve_btn.clicked() {
                self.pending_action = Some(PendingAction::Solve);
            }

            if any_busy {
                let cancel_btn =
                    ui.add(egui::Button::new("Cancel").min_size(egui::vec2(0.0, tokens::CONTROL)));
                self.note("Cancel", &cancel_btn);
                if cancel_btn.clicked() {
                    self.pending_action = Some(PendingAction::Cancel);
                }
            }
        });

        // Loading indicator while a background task is active.
        if any_busy {
            ui.add_space(tokens::XS);
            let label = if runner_busy {
                match self.run_kind {
                    Some(RunKind::Mesh) => "Meshing…",
                    Some(RunKind::Solve) | None => "Solving…",
                }
            } else if self.decompose_runner.running() {
                "Decomposing…"
            } else {
                "Updating plot…"
            };
            ui.horizontal(|ui| {
                ui.spinner();
                let lbl = ui.label(egui::RichText::new(label).color(p.text_muted));
                self.note("Loading", &lbl);
            });
        }

        ui.add_space(tokens::S);

        // Solution options bar (Contour, Mesh overlay, 3-D surface).
        if self.active_tab == MainTab::Solution {
            let mut opts_changed = false;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = tokens::XS;
                let c_btn = tokens::toggle(ui, self.solution_opts.contour, "Contour");
                self.note("Solution:Contour", &c_btn);
                if c_btn.clicked() {
                    self.solution_opts.contour = !self.solution_opts.contour;
                    opts_changed = true;
                }
                let m_btn = tokens::toggle(ui, self.solution_opts.mesh, "Mesh overlay");
                self.note("Solution:Mesh", &m_btn);
                if m_btn.clicked() {
                    self.solution_opts.mesh = !self.solution_opts.mesh;
                    opts_changed = true;
                }
                let s_btn = tokens::toggle(ui, self.solution_opts.surface_3d, "3-D surface");
                self.note("Solution:3D", &s_btn);
                if s_btn.clicked() {
                    self.solution_opts.surface_3d = !self.solution_opts.surface_3d;
                    opts_changed = true;
                }
            });
            if opts_changed && self.nodal_solution.is_some() {
                self.request_solution_replot(ui.ctx());
            }
            ui.add_space(tokens::XS);
        }

        // Reserve space for slider/mode picker and the 1-line info footer.
        let show_time_bar = self.active_tab == MainTab::Solution
            && matches!(
                self.model.pde_type,
                PdeType::Parabolic | PdeType::Hyperbolic
            )
            && self.tlist.as_ref().is_some_and(|t| t.len() >= 2);
        let show_eigen_bar = self.active_tab == MainTab::Solution
            && self.model.pde_type == PdeType::Eigen
            && self.eigenvalues.as_ref().is_some_and(|e| !e.is_empty());
        let extra_footer = if show_time_bar || show_eigen_bar {
            tokens::CONTROL + tokens::S
        } else {
            0.0
        };
        let info_footer = tokens::ROW + tokens::S;

        let body_h = (ui.available_height() - extra_footer - info_footer).max(200.0);
        let body_w = ui.available_width().max(120.0);

        match self.active_tab {
            MainTab::Draw => {
                ui.allocate_ui(egui::vec2(body_w, body_h), |ui| {
                    #[cfg(test)]
                    let outcome = self.canvas.show(ui, &mut self.model, &mut self.controls);
                    #[cfg(not(test))]
                    let outcome = self.canvas.show(ui, &mut self.model);

                    if outcome.geometry_changed {
                        self.mark_geometry_changed(true);
                    }
                    if let Some(eid) = outcome.edge_clicked {
                        self.scroll_to_edge = Some(eid);
                    }
                });
            }
            MainTab::Geometry => {
                ui.allocate_ui(egui::vec2(body_w, body_h), |ui| {
                    if let Some(view) = &mut self.geometry_view {
                        let id = egui::Id::new(("pde_modeler_geometry", view.serial));
                        views::show_prepared_figure(ui, view, id);
                    } else if !self.decompose_runner.running() {
                        tokens::helper(
                            ui,
                            "Draw shapes and click Decompose, Mesh, or Solve to view the geometry.",
                        );
                    }
                });
            }
            MainTab::Mesh => {
                ui.allocate_ui(egui::vec2(body_w, body_h), |ui| {
                    if let Some(view) = &mut self.mesh_view {
                        let id = egui::Id::new(("pde_modeler_mesh", view.serial));
                        views::show_prepared_figure(ui, view, id);
                    } else if !runner_busy {
                        tokens::helper(ui, "Click Mesh or Solve to generate the triangular mesh.");
                    }
                });
            }
            MainTab::Solution => {
                ui.allocate_ui(egui::vec2(body_w, body_h), |ui| {
                    if let Some(view) = &mut self.solution_view {
                        let id = egui::Id::new(("pde_modeler_solution", view.serial));
                        views::show_prepared_figure(ui, view, id);
                    } else if !runner_busy {
                        tokens::helper(
                            ui,
                            "Click Solve or choose a sample to compute and display the solution.",
                        );
                    }
                });
            }
        }

        // Time slider (Parabolic / Hyperbolic) under Solution view.
        if show_time_bar && let Some(tlist) = self.tlist.clone() {
            ui.add_space(tokens::XS);
            let max_idx = tlist.len() - 1;
            let mut idx = self.time_index.min(max_idx);
            let t_val = tlist[idx];
            ui.horizontal(|ui| {
                let play_lbl = if self.playing { "Pause" } else { "Play" };
                let play_btn =
                    ui.add(egui::Button::new(play_lbl).min_size(egui::vec2(56.0, tokens::CONTROL)));
                self.note("Solution:PlayPause", &play_btn);
                if play_btn.clicked() {
                    self.playing = !self.playing;
                    self.last_play_tick = self.playing.then(Instant::now);
                }
                let rail_idx = ui.painter().add(egui::Shape::Noop);
                let slider_resp = ui.add(
                    egui::Slider::new(&mut idx, 0..=max_idx)
                        .show_value(false)
                        .step_by(1.0)
                        .trailing_fill(true),
                );
                let rail_h = ui.spacing().slider_rail_height;
                let rail_rect = egui::Rect::from_center_size(
                    slider_resp.rect.center(),
                    egui::vec2(slider_resp.rect.width(), rail_h),
                );
                ui.painter().set(
                    rail_idx,
                    egui::Shape::rect_stroke(
                        rail_rect,
                        tokens::CONTROL_RADIUS as f32,
                        egui::Stroke::new(1.0, p.border),
                        egui::StrokeKind::Outside,
                    ),
                );
                self.note("Solution:TimeSlider", &slider_resp);
                let time_lbl = ui.label(
                    egui::RichText::new(format!("t = {t_val:.2}"))
                        .strong()
                        .color(p.text),
                );
                self.note("Solution:TimeLabel", &time_lbl);
            });
            if idx != self.time_index {
                self.playing = false;
                self.set_time_index(idx, ui.ctx());
            }
        }

        // Eigenmode picker (Eigen) under Solution view.
        if show_eigen_bar && let Some(eigs) = self.eigenvalues.clone() {
            ui.add_space(tokens::XS);
            let mut idx = self.eigen_index.min(eigs.len() - 1);
            let cur_lambda = eigs[idx];
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Mode").color(p.text));
                let combo = egui::ComboBox::from_id_salt("pde_eigen_mode")
                    .selected_text(format!("Mode {} (λ = {cur_lambda:.4})", idx + 1))
                    .show_ui(ui, |ui| {
                        for (k, &lam) in eigs.iter().enumerate() {
                            let r = ui.selectable_value(
                                &mut idx,
                                k,
                                format!("Mode {} (λ = {lam:.4})", k + 1),
                            );
                            self.note(&format!("Solution:Mode:{k}"), &r);
                        }
                    });
                self.note("Solution:ModePicker", &combo.response);
            });
            if idx != self.eigen_index {
                self.set_eigen_index(idx, ui.ctx());
            }
        }

        // Dedicated 1-line info line at the bottom of the Main view.
        ui.add_space(tokens::XS);
        let (line_text, is_error) = self.info_line_state();
        let info_color = if is_error { p.error } else { p.text_muted };
        #[cfg(test)]
        {
            self.last_info_color = Some(info_color);
        }
        let info_resp = ui.label(
            egui::RichText::new(line_text)
                .size(LABEL_SIZE)
                .color(info_color),
        );
        self.note("InfoLine", &info_resp);
    }
}

impl App for PdeModeler {
    fn kind(&self) -> AppKind {
        AppKind::PdeModeler
    }

    fn script(&self, _ws: &Snapshot) -> Result<String, String> {
        if self.bcs_stale {
            return Err("Boundary conditions are out of date; press Decompose".into());
        }
        self.validate_export_names()?;
        if let Some(err) = self.time_drafts_refusal() {
            return Err(err);
        }
        let base = self.model.script()?;
        Ok(views::rename_script_vars(
            &base,
            &self.export_names[0],
            &self.export_names[1],
        ))
    }

    /// Export runs the script in the workspace: drop its `pdem_` helper
    /// variables afterwards so only the two named outputs change.
    fn export_code(&self, ws: &Snapshot) -> Result<String, String> {
        let mut script = self.script(ws)?;
        let temps = if matches!(
            self.model.pde_type,
            PdeType::Parabolic | PdeType::Hyperbolic
        ) {
            "pdem_gd pdem_sf pdem_ns pdem_tlist"
        } else {
            "pdem_gd pdem_sf pdem_ns"
        };
        script.push_str(&format!("clear {temps}\n"));
        Ok(script)
    }

    fn inputs(&self) -> Vec<String> {
        Vec::new()
    }

    fn probe(&self, _ws: &Snapshot) -> (String, Vec<String>) {
        let [m_name, r_name] = &self.export_names;
        match self.model.pde_type {
            PdeType::Eigen => (
                format!(
                    "mrpde_u = {r_name}.Eigenvectors;\n\
                     mrpde_nodes = size({m_name}.Mesh.Nodes, 2);\n\
                     mrpde_elems = size({m_name}.Mesh.Elements, 2);\n"
                ),
                vec!["mrpde_u".into(), "mrpde_nodes".into(), "mrpde_elems".into()],
            ),
            PdeType::Elliptic | PdeType::Parabolic | PdeType::Hyperbolic => (
                format!(
                    "mrpde_u = {r_name}.NodalSolution;\n\
                     mrpde_nodes = size({m_name}.Mesh.Nodes, 2);\n\
                     mrpde_elems = size({m_name}.Mesh.Elements, 2);\n"
                ),
                vec!["mrpde_u".into(), "mrpde_nodes".into(), "mrpde_elems".into()],
            ),
        }
    }

    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let sol = outputs
            .get("mrpde_u")
            .cloned()
            .ok_or_else(|| "PDE Modeler: solver returned no NodalSolution".to_owned())?;
        let (min_u, max_u, time_steps) = compute_solution_stats(&sol);
        let scalar_usize = |key: &str| -> Option<usize> {
            match outputs.get(key) {
                Some(Array::Numeric(m)) => m.as_scalar().ok().map(|v| v.round() as usize),
                _ => None,
            }
        };
        self.nodal_solution = Some(sol);
        self.node_count = scalar_usize("mrpde_nodes");
        self.element_count = scalar_usize("mrpde_elems");
        self.min_u = min_u;
        self.max_u = max_u;
        self.time_steps = time_steps;
        Ok(())
    }

    fn inputs_ui(&mut self, ui: &mut egui::Ui, _ws: &Snapshot) {
        self.poll_decompose(ui.ctx());
        let p = tokens::palette(ui.ctx());

        ui.label(egui::RichText::new("Samples").strong().color(p.text));
        ui.add_space(tokens::XS);
        let hole_btn = ui.add(
            egui::Button::new("Plate with a hole")
                .min_size(egui::vec2(ui.available_width(), tokens::CONTROL)),
        );
        self.note("Plate with a hole", &hole_btn);
        if hole_btn.clicked() {
            self.pending_action = Some(PendingAction::SampleHole);
        }
        let heat_btn = ui.add(
            egui::Button::new("Heat in a plate")
                .min_size(egui::vec2(ui.available_width(), tokens::CONTROL)),
        );
        self.note("Heat in a plate", &heat_btn);
        if heat_btn.clicked() {
            self.pending_action = Some(PendingAction::SampleHeat);
        }

        ui.add_space(tokens::M);
        self.show_geometry_section(ui, p);
        self.show_bc_section(ui, p);
        self.show_pde_section(ui, p);
        self.show_mesh_section(ui, p);
    }

    fn main_caption(&self) -> &'static str {
        self.active_tab.caption()
    }

    fn main_ui(&mut self, ui: &mut egui::Ui) {
        self.show_main_area(ui, false);
    }

    fn results(&self) -> Vec<(&'static str, String)> {
        let mut rows = vec![
            ("Model", self.export_names[0].clone()),
            ("Results", self.export_names[1].clone()),
        ];
        if let Some(nodes) = self.node_count {
            rows.push(("Nodes", nodes.to_string()));
        }
        if let Some(elems) = self.element_count {
            rows.push(("Triangles", elems.to_string()));
        }
        if let Some(eigs) = &self.eigenvalues
            && !eigs.is_empty()
        {
            rows.push(("Modes", eigs.len().to_string()));
            rows.push(("First λ", format!("{:.4}", eigs[0])));
        } else if let Some(steps) = self.time_steps {
            rows.push(("Time steps", steps.to_string()));
        }
        let range = if self.model.pde_type == PdeType::Eigen {
            self.nodal_solution
                .as_ref()
                .and_then(|sol| column_range(sol, self.eigen_index))
        } else {
            self.min_u.zip(self.max_u)
        };
        if let Some((lo, hi)) = range {
            rows.push(("Range", format!("{lo:.4} .. {hi:.4}")));
        }
        rows
    }

    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let labels = ["Model variable", "Results variable"];
        for (label, name) in labels.iter().zip(self.export_names.iter_mut()) {
            super::name_field(ui, label, name, ws);
        }
    }

    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        self.export_names.to_vec()
    }

    fn summary(&self) -> Option<String> {
        let nodes = self.node_count?;
        let elems = self.element_count?;
        Some(format!(
            "PDE Modeler ({}): {nodes} nodes, {elems} triangles",
            self.model.pde_type.label()
        ))
    }
}

// ---------------------------------------------------------------------------
// Host-level runner orchestration and layout

pub(crate) fn show(
    host: &mut Host<PdeModeler>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    #[cfg(test)]
    host.app.controls.clear();

    refresh(host, ui.ctx(), ws);

    let mut export_clicked = false;
    let mut generate_clicked = false;
    let runner_busy = host.runner.running();
    let p = tokens::palette(ui.ctx());
    let frame = egui::Frame::new()
        .fill(p.surface_sunken)
        .inner_margin(tokens::L as i8);
    let narrow = ui.available_width() < 2.0 * super::COLUMN + 360.0;

    let mut show_output = |ui: &mut egui::Ui, host: &mut Host<PdeModeler>| {
        super::result_block(ui, &host.app.results());
        ui.add_space(tokens::M);
        host.app.names_ui(ui, ws);
        ui.add_space(tokens::S);
        let script_res = host.app.script(ws);
        let ready = !busy
            && !host.saving
            && !host.runner.running()
            && host.app.nodal_solution.is_some()
            && script_res.is_ok();
        let wide = egui::vec2(ui.available_width(), tokens::CONTROL);
        let exp_btn = ui.add_enabled(
            ready,
            egui::Button::new("Export to workspace").min_size(wide),
        );
        host.app.note("Export to workspace", &exp_btn);
        if exp_btn.clicked() {
            export_clicked = true;
        }
        let gen_btn = ui.add_enabled(ready, egui::Button::new("Generate script").min_size(wide));
        host.app.note("Generate script", &gen_btn);
        if gen_btn.clicked() {
            generate_clicked = true;
        }
    };

    if narrow {
        // Narrow width: inputs panel stacks above the view, then output below — no clipped controls.
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("pde_modeler_narrow_scroll")
                .auto_shrink([false, false])
                .show_themed(ui, |ui| {
                    ui.spacing_mut().interact_size.y = tokens::CONTROL;
                    section_caption(ui, "INPUTS");
                    host.app.inputs_ui(ui, ws);
                    ui.add_space(tokens::L);
                    section_caption(ui, host.app.main_caption());
                    let main_size = egui::vec2(ui.available_width(), 380.0);
                    ui.allocate_ui(main_size, |ui| {
                        ui.set_min_size(main_size);
                        host.app.show_main_area(ui, runner_busy);
                    });
                    ui.add_space(tokens::L);
                    section_caption(ui, "OUTPUT");
                    show_output(ui, host);
                });
        });
    } else {
        use crate::panels::PaneBorder;
        let id = |part: &str| egui::Id::new(("app_area", "PDE Modeler", part.to_owned()));
        egui::Panel::left(id("inputs"))
            .exact_size(super::COLUMN)
            .resizable(false)
            .frame(frame)
            .show_pane(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(id("inputs_scroll"))
                    .auto_shrink([false, false])
                    .show_themed(ui, |ui| {
                        ui.spacing_mut().interact_size.y = tokens::CONTROL;
                        section_caption(ui, "INPUTS");
                        host.app.inputs_ui(ui, ws);
                    });
            });
        egui::Panel::right(id("output"))
            .exact_size(super::COLUMN)
            .resizable(false)
            .frame(frame)
            .show_pane(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(id("output_scroll"))
                    .auto_shrink([false, false])
                    .show_themed(ui, |ui| {
                        ui.spacing_mut().interact_size.y = tokens::CONTROL;
                        section_caption(ui, "OUTPUT");
                        show_output(ui, host);
                    });
            });
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            section_caption(ui, host.app.main_caption());
            host.app.show_main_area(ui, runner_busy);
        });
    }

    if export_clicked {
        *action = host.export(ws);
        // Host::export stores the notice on `host.message` only. The info
        // line is the one place PDE Modeler shows it (not the output panel).
        if action.is_some() {
            host.app.set_info(host.message.clone());
        }
    }
    if generate_clicked {
        *action = host.generate(ws, super::now());
    }

    if let Some(act) = host.app.pending_action.take() {
        match act {
            PendingAction::Mesh => start_mesh(host, ui.ctx(), ws),
            PendingAction::Solve => start_solve(host, ui.ctx(), ws),
            PendingAction::Cancel => cancel_running(host),
            PendingAction::SampleHole => {
                host.app.load_plate_with_hole();
                host.app.active_tab = MainTab::Solution;
                start_solve(host, ui.ctx(), ws);
            }
            PendingAction::SampleHeat => {
                host.app.load_plate_heat();
                host.app.active_tab = MainTab::Solution;
                start_solve(host, ui.ctx(), ws);
            }
        }
    }

    refresh(host, ui.ctx(), ws);
}

fn section_caption(ui: &mut egui::Ui, text: &str) {
    let p = tokens::palette(ui.ctx());
    ui.label(
        egui::RichText::new(text)
            .font(crate::panels::caption_semibold(ui))
            .color(p.text),
    );
    ui.add_space(tokens::S);
}

pub(crate) fn refresh(host: &mut Host<PdeModeler>, ctx: &egui::Context, ws: &Snapshot) {
    host.set_folder(ws.folder());
    host.runner.poll(ctx);
    host.app.poll_decompose(ctx);
    host.app.poll_plot(ctx);

    let fp = host.app.model_fingerprint();
    let model_changed = host
        .app
        .last_model_fingerprint
        .is_some_and(|prev| prev != fp);
    host.app.last_model_fingerprint = Some(fp);
    if model_changed || host.app.export_notice_stale {
        host.app.export_notice_stale = false;
        // The info line holds a copy of this notice. Drop both together so
        // an old "Exported…" line does not survive the model change.
        let cleared_notice = if host.message.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut host.message))
        };
        if let Some(notice) = cleared_notice.as_deref()
            && !host.app.info_is_error
            && host.app.info_message.as_deref() == Some(notice)
        {
            host.app.info_message = None;
        }
        if model_changed && host.app.info_is_error {
            host.app.info_message = None;
            host.app.info_is_error = false;
            host.app.decompose_error = None;
        }
    }

    // Refit canvas if shapes were added programmatically outside current view.
    if host.app.model.shapes.len() > host.app.last_shape_count {
        host.app
            .canvas
            .view
            .refit_if_outside(&host.app.model.shapes);
        host.app.last_shape_count = host.app.model.shapes.len();
    } else if host.app.model.shapes.len() < host.app.last_shape_count {
        host.app.last_shape_count = host.app.model.shapes.len();
    }

    // Play animation for time-dependent solutions.
    if host.app.playing
        && !host.app.plot_runner.running()
        && let Some(tlist) = &host.app.tlist
        && tlist.len() >= 2
    {
        let now = Instant::now();
        let step_due = host
            .app
            .last_play_tick
            .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(120));
        if step_due {
            host.app.last_play_tick = Some(now);
            let next = (host.app.time_index + 1) % tlist.len();
            host.app.set_time_index(next, ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(60));
    }

    // Deliver finished Mesh / Solve run from host.runner.
    if host.runner.key() != host.delivered
        && let Some(result) = host.runner.result().cloned()
    {
        host.delivered = host.runner.key();
        let kind = host.app.run_kind.take();
        let guard = host.app.run_guard.take();
        let payload = host
            .app
            .run_payload
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        let stale = match (kind, guard) {
            (Some(k), Some(g)) => host.app.run_guard(k) != g,
            _ => false,
        };

        if stale {
            let what = match kind {
                Some(RunKind::Mesh) => "Mesh",
                _ => "Solve",
            };
            host.app.set_info(format!(
                "{what} result discarded because the model changed; press {what} again."
            ));
        } else {
            match result {
                Ok(_) => {
                    if let Some(payload) = payload {
                        apply_run_payload(&mut host.app, payload);
                    }
                }
                Err(err) => {
                    let line = views::first_line(&err);
                    host.app.set_error(line);
                    match kind {
                        Some(RunKind::Mesh) => host.app.clear_mesh_and_solution(),
                        Some(RunKind::Solve) | None => host.app.clear_solution(),
                    }
                }
            }
        }
    }

    // Sync host status fields so `host.status()` stays authoritative.
    if host.app.info_is_error
        && let Some(err) = &host.app.info_message
    {
        host.script_error = None;
        host.run_error = Some(err.clone());
    } else if host.app.model.shapes.is_empty() {
        host.script_error = Some("No shape drawn. Add a shape or load a sample.".into());
        host.run_error = None;
    } else if host.app.bcs_stale {
        host.script_error = Some("Boundary conditions are out of date; press Decompose".into());
        host.run_error = None;
    } else {
        host.script_error = None;
        host.run_error = None;
    }
}

fn apply_run_payload(app: &mut PdeModeler, payload: RunPayload) {
    if let Some(dl) = payload.dl {
        let edges = canvas::decode_dl_edges(&dl);
        let ids: Vec<usize> = edges.iter().filter(|e| e.boundary).map(|e| e.id).collect();
        if app.model.edge_ids.is_empty() {
            app.model.edge_ids = ids;
        }
        app.canvas.decomposed_edges = edges;
    }
    if let Some(g) = payload.geometry_view {
        app.geometry_view = Some(g);
    }
    if let Some(m) = payload.mesh_view {
        app.mesh_view = Some(m);
    }
    if let Some(nodes) = payload.mesh_nodes {
        app.mesh_nodes = Some(nodes);
    }
    if let Some(elems) = payload.mesh_elems {
        app.mesh_elems = Some(elems);
    }
    app.node_count = payload.node_count;
    app.element_count = payload.element_count;
    app.mesh_signature = app.mesh_signature_now();

    match payload.kind {
        RunKind::Mesh => {
            if let (Some(n), Some(e)) = (app.node_count, app.element_count) {
                app.set_info(format!("{n} nodes, {e} triangles"));
            }
            app.active_tab = MainTab::Mesh;
        }
        RunKind::Solve => {
            if let Some(sol) = payload.nodal_solution {
                let (min_u, max_u, steps) = if app.model.pde_type == PdeType::Eigen {
                    let mode_range = column_range(&sol, 0);
                    (mode_range.map(|r| r.0), mode_range.map(|r| r.1), None)
                } else {
                    compute_solution_stats(&sol)
                };
                app.nodal_solution = Some(sol);
                app.min_u = min_u;
                app.max_u = max_u;
                app.time_steps = steps;
            }
            app.eigenvalues = payload.eigenvalues;
            app.eigen_index = 0;
            app.tlist = payload.tlist;
            app.time_index = 0;
            app.playing = false;
            app.solution_view = payload.solution_view;
            app.info_message = None;
            app.info_is_error = false;
            app.active_tab = MainTab::Solution;
        }
    }
}

pub(crate) fn start_mesh(host: &mut Host<PdeModeler>, ctx: &egui::Context, ws: &Snapshot) {
    host.set_folder(ws.folder());
    if host.app.bcs_stale {
        host.app
            .set_info("Boundary conditions are out of date; press Decompose");
        return;
    }
    let geo_script = match host.app.model.geometry_script() {
        Ok(s) => s,
        Err(err) => {
            host.app.set_error(err);
            host.app.clear_all_views();
            return;
        }
    };
    let hmax = host.app.model.mesh.hmax.trim();
    if !hmax.is_empty() {
        match hmax.parse::<f64>() {
            Ok(v) if v.is_finite() && v > 0.0 => {}
            _ => {
                host.app
                    .set_error(format!("Invalid Hmax '{hmax}': must be a positive number."));
                host.app.clear_mesh_and_solution();
                return;
            }
        }
    }
    let order = host.app.model.mesh.order.as_str();
    let mesh_line = if hmax.is_empty() {
        format!("generateMesh(model, 'GeometricOrder', '{order}');\n")
    } else {
        format!("generateMesh(model, 'Hmax', {hmax}, 'GeometricOrder', '{order}');\n")
    };
    let code = format!(
        "{geo_script}\
         mrpde_dl = decsg(pdem_gd, pdem_sf, pdem_ns);\n\
         {mesh_line}\
         mrpde_nodes = size(model.Mesh.Nodes, 2);\n\
         mrpde_elems = size(model.Mesh.Elements, 2);\n\
         mrpde_mesh_nodes = model.Mesh.Nodes;\n\
         mrpde_mesh_elems = model.Mesh.Elements;\n\
         figure(1); pdegplot(model, 'EdgeLabels', 'on', 'FaceLabels', 'on');\n\
         figure(2); pdemesh(model);\n"
    );

    host.runner.stop();
    host.app.plot_runner.stop();
    host.delivered = None;
    host.app.clear_mesh_and_solution();
    host.app.info_message = None;
    host.app.info_is_error = false;
    host.app.mesh_count += 1;
    host.app.run_seq += 1;
    let seq = host.app.run_seq;
    host.app.run_kind = Some(RunKind::Mesh);
    host.app.run_guard = Some(host.app.run_guard(RunKind::Mesh));

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seq.hash(&mut hasher);
    code.hash(&mut hasher);
    let key = hasher.finish();
    let request = Request {
        key,
        inputs: Vec::new(),
        code,
        outputs: vec![
            "mrpde_dl".into(),
            "mrpde_nodes".into(),
            "mrpde_elems".into(),
            "mrpde_mesh_nodes".into(),
            "mrpde_mesh_elems".into(),
        ],
    };
    let payload_slot = host.app.run_payload.clone();
    #[cfg(test)]
    let delay_ms = host.app.test_mesh_delay_ms;
    host.runner.submit_task(
        ctx,
        key,
        Box::new(move |engine| {
            #[cfg(test)]
            test_delay(engine, delay_ms)?;
            let outputs = super::run(engine, request)?;
            let cancel = engine.cancellation();
            let mut figs: BTreeMap<u32, matrust::plot::Figure> =
                engine.figures().map(|(id, f)| (id, f.clone())).collect();
            let geometry_view = match figs.remove(&1) {
                Some(f) => Some(views::prepare_figure(f, &cancel, seq * 4 + 1)?),
                None => None,
            };
            let mesh_view = match figs.remove(&2) {
                Some(f) => Some(views::prepare_figure(f, &cancel, seq * 4 + 2)?),
                None => None,
            };
            let dl = match outputs.get("mrpde_dl") {
                Some(Array::Numeric(m)) => Some(m.clone()),
                _ => None,
            };
            let payload = RunPayload {
                kind: RunKind::Mesh,
                geometry_view,
                mesh_view,
                solution_view: None,
                dl,
                mesh_nodes: outputs.get("mrpde_mesh_nodes").cloned(),
                mesh_elems: outputs.get("mrpde_mesh_elems").cloned(),
                node_count: extract_scalar_usize(&outputs, "mrpde_nodes"),
                element_count: extract_scalar_usize(&outputs, "mrpde_elems"),
                nodal_solution: None,
                eigenvalues: None,
                tlist: None,
            };
            *payload_slot
                .lock()
                .map_err(|_| "Mesh delivery lock failed")? = Some(payload);
            Ok(outputs)
        }),
    );
}

pub(crate) fn start_solve(host: &mut Host<PdeModeler>, ctx: &egui::Context, ws: &Snapshot) {
    host.set_folder(ws.folder());
    if host.app.bcs_stale {
        host.app
            .set_info("Boundary conditions are out of date; press Decompose");
        return;
    }
    if let Some(err) = host.app.time_drafts_refusal() {
        host.app.snap_time_drafts();
        host.app.set_error(err);
        host.app.clear_solution();
        return;
    }
    if let Err(err) = host.app.model.validate() {
        host.app.set_error(err);
        host.app.clear_solution();
        return;
    }

    // DOF guard before time-dependent solves (no engine call when violated).
    let is_time_dep = matches!(
        host.app.model.pde_type,
        PdeType::Parabolic | PdeType::Hyperbolic
    );
    if is_time_dep {
        let equations = 1usize;
        if host.app.mesh_is_current()
            && let Some(nodes) = host.app.node_count
            && nodes.saturating_mul(equations) > TIME_DOF_LIMIT
        {
            host.app.set_error(format!(
                "Mesh has {nodes} nodes; time-dependent solves are limited to {TIME_DOF_LIMIT}. Increase Hmax."
            ));
            host.app.clear_solution();
            return;
        }
    }

    let base_script = match host.app.model.script() {
        Ok(s) => s,
        Err(err) => {
            host.app.set_error(err);
            host.app.clear_solution();
            return;
        }
    };

    // Inject post-mesh DOF guard right before `solvepde(model, pdem_tlist)` for auto/borderline Hmax.
    let solve_script = if is_time_dep {
        let guard = format!(
            "mrpde_dof_nodes = size(model.Mesh.Nodes, 2);\n\
             if mrpde_dof_nodes > {TIME_DOF_LIMIT}\n\
                 error('Mesh has %d nodes; time-dependent solves are limited to {TIME_DOF_LIMIT}. Increase Hmax.', mrpde_dof_nodes);\n\
             end\n\
             results = solvepde(model, pdem_tlist);"
        );
        base_script.replace("results = solvepde(model, pdem_tlist);", &guard)
    } else {
        base_script
    };

    let pde_type = host.app.model.pde_type;
    let opts = host.app.solution_opts;
    let plot_call = views::pdeplot_call("model", "mrpde_u(:, 1)", opts);
    let mut outputs_list = vec![
        "mrpde_dl".into(),
        "mrpde_nodes".into(),
        "mrpde_elems".into(),
        "mrpde_mesh_nodes".into(),
        "mrpde_mesh_elems".into(),
        "mrpde_u".into(),
    ];
    let tail = match pde_type {
        PdeType::Eigen => {
            outputs_list.push("mrpde_eig".into());
            format!(
                "\nmrpde_dl = decsg(pdem_gd, pdem_sf, pdem_ns);\n\
                 mrpde_nodes = size(model.Mesh.Nodes, 2);\n\
                 mrpde_elems = size(model.Mesh.Elements, 2);\n\
                 mrpde_mesh_nodes = model.Mesh.Nodes;\n\
                 mrpde_mesh_elems = model.Mesh.Elements;\n\
                 mrpde_u = results.Eigenvectors;\n\
                 mrpde_eig = results.Eigenvalues;\n\
                 if isempty(mrpde_eig)\n\
                     error('No eigenvalues found in the requested range.');\n\
                 end\n\
                 figure(1); pdegplot(model, 'EdgeLabels', 'on', 'FaceLabels', 'on');\n\
                 figure(2); pdemesh(model);\n\
                 figure(3); {plot_call}"
            )
        }
        PdeType::Parabolic | PdeType::Hyperbolic => {
            outputs_list.push("mrpde_tlist".into());
            format!(
                "\nmrpde_dl = decsg(pdem_gd, pdem_sf, pdem_ns);\n\
                 mrpde_nodes = size(model.Mesh.Nodes, 2);\n\
                 mrpde_elems = size(model.Mesh.Elements, 2);\n\
                 mrpde_mesh_nodes = model.Mesh.Nodes;\n\
                 mrpde_mesh_elems = model.Mesh.Elements;\n\
                 mrpde_u = results.NodalSolution;\n\
                 mrpde_tlist = pdem_tlist;\n\
                 figure(1); pdegplot(model, 'EdgeLabels', 'on', 'FaceLabels', 'on');\n\
                 figure(2); pdemesh(model);\n\
                 figure(3); {plot_call}"
            )
        }
        PdeType::Elliptic => format!(
            "\nmrpde_dl = decsg(pdem_gd, pdem_sf, pdem_ns);\n\
             mrpde_nodes = size(model.Mesh.Nodes, 2);\n\
             mrpde_elems = size(model.Mesh.Elements, 2);\n\
             mrpde_mesh_nodes = model.Mesh.Nodes;\n\
             mrpde_mesh_elems = model.Mesh.Elements;\n\
             mrpde_u = results.NodalSolution;\n\
             figure(1); pdegplot(model, 'EdgeLabels', 'on', 'FaceLabels', 'on');\n\
             figure(2); pdemesh(model);\n\
             figure(3); {plot_call}"
        ),
    };
    let code = format!("{solve_script}{tail}");

    host.runner.stop();
    host.app.plot_runner.stop();
    host.delivered = None;
    host.app.clear_solution();
    host.app.info_message = None;
    host.app.info_is_error = false;
    host.app.solve_count += 1;
    host.app.run_seq += 1;
    let seq = host.app.run_seq;
    host.app.run_kind = Some(RunKind::Solve);
    host.app.run_guard = Some(host.app.run_guard(RunKind::Solve));

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seq.hash(&mut hasher);
    code.hash(&mut hasher);
    let key = hasher.finish();
    let request = Request {
        key,
        inputs: Vec::new(),
        code,
        outputs: outputs_list,
    };
    let payload_slot = host.app.run_payload.clone();
    #[cfg(test)]
    let delay_ms = host.app.test_solve_delay_ms;

    host.runner.submit_task(
        ctx,
        key,
        Box::new(move |engine| {
            #[cfg(test)]
            test_delay(engine, delay_ms)?;
            let outputs = super::run(engine, request)?;
            let fixed_clim = if is_time_dep {
                outputs.get("mrpde_u").and_then(views::global_solution_clim)
            } else {
                None
            };
            if let Some([lo, hi]) = fixed_clim {
                engine.execute(&format!("figure(3); clim([{lo:?}, {hi:?}]);"))?;
            }
            let cancel = engine.cancellation();
            let mut figs: BTreeMap<u32, matrust::plot::Figure> =
                engine.figures().map(|(id, f)| (id, f.clone())).collect();
            let geometry_view = match figs.remove(&1) {
                Some(f) => Some(views::prepare_figure(f, &cancel, seq * 4 + 1)?),
                None => None,
            };
            let mesh_view = match figs.remove(&2) {
                Some(f) => Some(views::prepare_figure(f, &cancel, seq * 4 + 2)?),
                None => None,
            };
            let solution_view = match figs.remove(&3) {
                Some(f) => Some(views::prepare_figure(f, &cancel, seq * 4 + 3)?),
                None => None,
            };
            let dl = match outputs.get("mrpde_dl") {
                Some(Array::Numeric(m)) => Some(m.clone()),
                _ => None,
            };
            let eigenvalues = match outputs.get("mrpde_eig") {
                Some(Array::Numeric(m)) => Some(m.data().to_vec()),
                _ => None,
            };
            let tlist = match outputs.get("mrpde_tlist") {
                Some(Array::Numeric(m)) => Some(m.data().to_vec()),
                _ => None,
            };
            let payload = RunPayload {
                kind: RunKind::Solve,
                geometry_view,
                mesh_view,
                solution_view,
                dl,
                mesh_nodes: outputs.get("mrpde_mesh_nodes").cloned(),
                mesh_elems: outputs.get("mrpde_mesh_elems").cloned(),
                node_count: extract_scalar_usize(&outputs, "mrpde_nodes"),
                element_count: extract_scalar_usize(&outputs, "mrpde_elems"),
                nodal_solution: outputs.get("mrpde_u").cloned(),
                eigenvalues,
                tlist,
            };
            *payload_slot
                .lock()
                .map_err(|_| "Solve delivery lock failed")? = Some(payload);
            Ok(outputs)
        }),
    );
}

/// Test-only cancellable delay before a Mesh/Solve job runs.
#[cfg(test)]
fn test_delay(engine: &mut matrust::Engine, delay_ms: u64) -> Result<(), String> {
    if delay_ms == 0 {
        return Ok(());
    }
    let cancel = engine.cancellation();
    let start = Instant::now();
    let target = Duration::from_millis(delay_ms);
    while start.elapsed() < target {
        if cancel.load(Ordering::Relaxed) {
            return Err("Operation cancelled.".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

pub(crate) fn cancel_running(host: &mut Host<PdeModeler>) {
    host.runner.stop();
    host.delivered = None;
    host.app.decompose_runner.stop();
    host.app.decompose_delivered = None;
    host.app.plot_runner.stop();
    host.app.plot_delivered = None;
    host.app.run_kind = None;
    host.app.run_guard = None;
    host.app.playing = false;
    host.app.set_info("Operation cancelled.");
}

pub(crate) fn close(host: &mut Host<PdeModeler>) {
    host.runner.stop();
    host.app.decompose_runner.stop();
    host.app.plot_runner.stop();
}

pub(crate) fn receive_open(workspace: &mut crate::Workspace) {
    let launch = workspace
        .worker
        .pde_modeler
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if let Some(launch) = launch
        && !workspace.worker.cancel.load(Ordering::Relaxed)
    {
        let tool = crate::toolkits::Tool::App(AppKind::PdeModeler);
        match launch {
            Launch::Open => workspace.open_app(tool),
            Launch::Close => workspace.close_tool_tab(tool),
        }
    }
}

fn column_range(sol: &Array, col_idx: usize) -> Option<(f64, f64)> {
    let Array::Numeric(m) = sol else {
        return None;
    };
    if col_idx >= m.cols() {
        return None;
    }
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for r in 0..m.rows() {
        let v = m.get(r, col_idx);
        if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
}

fn compute_solution_stats(sol: &Array) -> (Option<f64>, Option<f64>, Option<usize>) {
    match sol {
        Array::Numeric(m) => {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for &v in m.data() {
                if v.is_finite() {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            let range = if lo.is_finite() && hi.is_finite() {
                (Some(lo), Some(hi))
            } else {
                (None, None)
            };
            let steps = (m.cols() > 1).then_some(m.cols());
            (range.0, range.1, steps)
        }
        _ => (None, None, None),
    }
}

fn extract_scalar_usize(outputs: &Outputs, key: &str) -> Option<usize> {
    match outputs.get(key) {
        Some(Array::Numeric(m)) => m.as_scalar().ok().map(|v| v.round() as usize),
        _ => None,
    }
}

/// Replace `old` as a whole identifier token in a set formula.
pub(crate) fn replace_formula_token(formula: &str, old: &str, new: &str) -> String {
    if old.is_empty() || old == new {
        return formula.to_owned();
    }
    let chars: Vec<char> = formula.chars().collect();
    let old_chars: Vec<char> = old.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let boundary_before = i == 0 || !is_formula_ident(chars[i - 1]);
        let end = i + old_chars.len();
        if boundary_before
            && end <= chars.len()
            && chars[i..end] == old_chars[..]
            && (end == chars.len() || !is_formula_ident(chars[end]))
        {
            out.push_str(new);
            i = end;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn is_formula_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn fmt_field(v: f64) -> String {
    if v == 0.0 { "0".into() } else { format!("{v}") }
}

fn is_zero_or_empty(s: &str) -> bool {
    let t = s.trim();
    t.is_empty() || t == "0" || t == "0.0"
}
