//! 2-D PDE model handle (`createpde`), analytic geometry attachment
//! (`geometryFromEdges`), and geometry plotting (`pdegplot`).
use super::{
    Engine, Execution, Value, objects,
    pde_geometry::{GEdge, GKind, Geometry2},
};
use crate::{Array, GraphicsArray, GraphicsHandle, LogicalMatrix, Matrix, Result, StructArray};
use std::collections::HashMap;
use std::f64::consts::FRAC_PI_2;
use std::sync::Arc;

/// Upper bound on PDESystemSize N in `createpde(N)`.
const MAX_SYSTEM_SIZE: usize = 1024;
/// Upper bound on sample segments per arc when plotting.
const MAX_ARC_STEPS: usize = 4096;
/// Minimum sample points per quarter circle (pi/2) for arcs in `pdegplot`.
const ARC_STEPS_PER_QUARTER: f64 = 32.0;

pub(crate) const MODEL_CLASS: &str = "pde.PDEModel";
pub(crate) const GEOMETRY_CLASS: &str = "pde.AnalyticGeometry";

pub(crate) const MODEL_PROPERTIES: [&str; 8] = [
    "PDESystemSize",
    "IsTimeDependent",
    "Geometry",
    "Mesh",
    "EquationCoefficients",
    "BoundaryConditions",
    "InitialConditions",
    "SolverOptions",
];

pub(crate) const GEOMETRY_PROPERTIES: [&str; 4] =
    ["NumFaces", "NumEdges", "NumVertices", "Vertices"];

pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "createpde"
            | "geometryFromEdges"
            | "pdegplot"
            | "generateMesh"
            | "findNodes"
            | "findElements"
            | "pdemesh"
            | "pdeplot"
            | "specifyCoefficients"
            | "applyBoundaryCondition"
            | "setInitialConditions"
            | "solvepde"
            | "solvepdeeig"
            | "interpolateSolution"
            | "evaluateGradient"
    )
}

pub(super) fn bare(name: &str) -> bool {
    name == "createpde"
}

pub(super) fn statement(name: &str) -> bool {
    matches!(name, "pdegplot" | "pdemesh" | "pdeplot")
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let max_out = if name == "evaluateGradient" { 2 } else { 1 };
    if count > max_out {
        return Err(format!("{name}: too many output arguments"));
    }
    match name {
        "createpde" => {
            let model = createpde(e, args)?;
            Ok(if count == 0 { vec![] } else { vec![model] })
        }
        "geometryFromEdges" => {
            let geom = geometry_from_edges(e, args)?;
            Ok(if count == 0 { vec![] } else { vec![geom] })
        }
        "pdegplot" => {
            let lines = pdegplot(e, args, out)?;
            Ok(if count == 0 { vec![] } else { vec![lines] })
        }
        "generateMesh" | "findNodes" | "findElements" | "pdemesh" => {
            super::pde_mesh::call(e, name, args, count, out)
        }
        "pdeplot" => super::pde_plot::call(e, name, args, count, out),
        "specifyCoefficients"
        | "applyBoundaryCondition"
        | "setInitialConditions"
        | "solvepde"
        | "solvepdeeig"
        | "interpolateSolution"
        | "evaluateGradient" => super::pde_solve::call(e, name, args, count, out),
        _ => Err(format!("Unknown PDE model function {name}")),
    }
}

// ---------------------------------------------------------------------------
// pde.AnalyticGeometry (value semantics)

/// Validated 2-D analytic geometry with deduplicated vertices in deterministic
/// first-appearance order along `dl` columns.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AnalyticGeometry {
    pub(crate) geom: Geometry2,
    pub(crate) vertices: Vec<[f64; 2]>,
    pub(crate) edge_vertices: Vec<(usize, usize)>,
}

impl AnalyticGeometry {
    pub(crate) fn from_geometry2(geom: Geometry2) -> Self {
        let mut scale = 0.0f64;
        for edge in &geom.edges {
            for p in [edge.start, edge.end] {
                scale = scale.max(p[0].abs()).max(p[1].abs());
            }
        }
        let tol = 1e-12 * scale.max(f64::MIN_POSITIVE);
        let cell = (tol * 4.0).max(f64::MIN_POSITIVE);
        let mut vertices: Vec<[f64; 2]> = Vec::new();
        let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        let mut vid = |p: [f64; 2]| -> usize {
            let key = ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    if let Some(list) = grid.get(&(key.0 + dx, key.1 + dy)) {
                        for &i in list {
                            let q = vertices[i];
                            if (q[0] - p[0]).abs() <= tol && (q[1] - p[1]).abs() <= tol {
                                return i;
                            }
                        }
                    }
                }
            }
            let idx = vertices.len();
            vertices.push(p);
            grid.entry(key).or_default().push(idx);
            idx
        };
        let mut edge_vertices = Vec::with_capacity(geom.edges.len());
        for edge in &geom.edges {
            let a = vid(edge.start);
            let b = vid(edge.end);
            edge_vertices.push((a, b));
        }
        Self {
            geom,
            vertices,
            edge_vertices,
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.geom.edges.len() * std::mem::size_of::<GEdge>()
            + self.vertices.len() * std::mem::size_of::<[f64; 2]>()
            + self.edge_vertices.len() * std::mem::size_of::<(usize, usize)>()
    }

    pub(crate) fn property(&self, name: &str) -> Result<Value> {
        match name {
            "NumFaces" => Ok(Value::Matrix(Matrix::scalar(self.geom.num_faces as f64))),
            "NumEdges" => Ok(Value::Matrix(Matrix::scalar(self.geom.edges.len() as f64))),
            "NumVertices" => Ok(Value::Matrix(Matrix::scalar(self.vertices.len() as f64))),
            "Vertices" => {
                let n = self.vertices.len();
                let mut data = Vec::with_capacity(n * 2);
                data.extend(self.vertices.iter().map(|p| p[0]));
                data.extend(self.vertices.iter().map(|p| p[1]));
                Ok(Value::Matrix(Matrix::from_column_vec(n, 2, data)?))
            }
            _ => Err(format!(
                "No appropriate method, property, or field '{name}' for class '{GEOMETRY_CLASS}'."
            )),
        }
    }

    pub(crate) fn property_ci(&self, name: &str) -> Result<Value> {
        if let Some(&canonical) = GEOMETRY_PROPERTIES
            .iter()
            .find(|p| p.eq_ignore_ascii_case(name))
        {
            self.property(canonical)
        } else {
            self.property(name)
        }
    }

    pub(crate) fn get_method(&self, e: &Engine, rest: &[Value]) -> Result<Value> {
        match rest {
            [] => {
                let mut fields = Vec::with_capacity(GEOMETRY_PROPERTIES.len());
                for &prop in &GEOMETRY_PROPERTIES {
                    let val = self.property(prop)?.array(&e.cancel)?;
                    fields.push((prop.to_owned(), Arc::new(val)));
                }
                let names = fields.iter().map(|(n, _)| n.clone()).collect();
                let slots = fields.into_iter().map(|(_, v)| v).collect();
                Ok(Value::Container(Array::Struct(StructArray::from_slots(
                    1, 1, names, slots, &e.cancel,
                )?)))
            }
            [prop] => {
                let name =
                    text_value(prop).ok_or("pde.AnalyticGeometry: property name must be text")?;
                self.property_ci(&name)
            }
            _ => Err("pde.AnalyticGeometry: get expects zero or one property name".into()),
        }
    }

    pub(crate) fn body(&self) -> String {
        format!(
            "  AnalyticGeometry with properties:\n\n\
             {:>15}: {}\n\
             {:>15}: {}\n\
             {:>15}: {}\n\
             {:>15}: [{}x2 double]\n",
            "NumFaces",
            self.geom.num_faces,
            "NumEdges",
            self.geom.edges.len(),
            "NumVertices",
            self.vertices.len(),
            "Vertices",
            self.vertices.len(),
        )
    }
}

// ---------------------------------------------------------------------------
// pde.PDEModel (mutable handle semantics)

#[derive(Clone)]
pub(crate) struct PdeModel {
    pub(crate) system_size: usize,
    pub(crate) is_time_dependent: bool,
    pub(crate) geometry: Option<(Arc<AnalyticGeometry>, Arc<Array>)>,
    pub(crate) mesh: Option<Arc<Array>>,
    pub(crate) equation_coefficients: Option<Arc<Array>>,
    pub(crate) boundary_conditions: Option<Arc<Array>>,
    pub(crate) initial_conditions: Option<Arc<Array>>,
    pub(crate) solver_options: super::pde_solve::types::SolverOptions,
    pub(crate) coeff_assignments: Vec<super::pde_solve::types::CoeffAssignment>,
    pub(crate) bc_assignments: Vec<super::pde_solve::types::BcAssignment>,
    pub(crate) ic_assignments: Vec<super::pde_solve::types::IcAssignment>,
}

impl PdeModel {
    pub(crate) fn new(system_size: usize) -> Self {
        Self {
            system_size,
            is_time_dependent: false,
            geometry: None,
            mesh: None,
            equation_coefficients: None,
            boundary_conditions: None,
            initial_conditions: None,
            solver_options: super::pde_solve::types::SolverOptions::default(),
            coeff_assignments: Vec::new(),
            bc_assignments: Vec::new(),
            ic_assignments: Vec::new(),
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.geometry.as_ref().map_or(0, |(g, _)| g.bytes())
            + self.mesh.as_ref().map_or(0, |a| a.retained_bytes())
            + self
                .equation_coefficients
                .as_ref()
                .map_or(0, |a| a.retained_bytes())
            + self
                .boundary_conditions
                .as_ref()
                .map_or(0, |a| a.retained_bytes())
            + self
                .initial_conditions
                .as_ref()
                .map_or(0, |a| a.retained_bytes())
            + self
                .coeff_assignments
                .iter()
                .map(super::pde_solve::types::CoeffAssignment::bytes)
                .sum::<usize>()
            + self
                .bc_assignments
                .iter()
                .map(super::pde_solve::types::BcAssignment::bytes)
                .sum::<usize>()
            + self
                .ic_assignments
                .iter()
                .map(super::pde_solve::types::IcAssignment::bytes)
                .sum::<usize>()
    }

    pub(crate) fn push_children<'a>(&'a self, pending: &mut Vec<&'a Array>) {
        if let Some((_, a)) = &self.geometry {
            pending.push(a.as_ref());
        }
        if let Some(a) = &self.mesh {
            pending.push(a.as_ref());
        }
        if let Some(a) = &self.equation_coefficients {
            pending.push(a.as_ref());
        }
        if let Some(a) = &self.boundary_conditions {
            pending.push(a.as_ref());
        }
        if let Some(a) = &self.initial_conditions {
            pending.push(a.as_ref());
        }
        for ca in &self.coeff_assignments {
            ca.push_children(pending);
        }
        for bc in &self.bc_assignments {
            bc.push_children(pending);
        }
        for ic in &self.ic_assignments {
            ic.push_children(pending);
        }
    }

    pub(crate) fn property(&self, name: &str) -> Result<Value> {
        let empty = || Ok(Value::Matrix(Matrix::filled(0, 0, 0.0)?));
        match name {
            "PDESystemSize" => Ok(Value::Matrix(Matrix::scalar(self.system_size as f64))),
            "IsTimeDependent" => Ok(Value::Logical(LogicalMatrix::scalar(
                self.is_time_dependent,
            ))),
            "Geometry" => match &self.geometry {
                Some((_, arr)) => Ok(Value::Container(arr.as_ref().clone())),
                None => empty(),
            },
            "Mesh" => match &self.mesh {
                Some(arr) => Ok(Value::Container(arr.as_ref().clone())),
                None => empty(),
            },
            "EquationCoefficients" => match &self.equation_coefficients {
                Some(arr) => Ok(Value::Container(arr.as_ref().clone())),
                None => empty(),
            },
            "BoundaryConditions" => match &self.boundary_conditions {
                Some(arr) => Ok(Value::Container(arr.as_ref().clone())),
                None => empty(),
            },
            "InitialConditions" => match &self.initial_conditions {
                Some(arr) => Ok(Value::Container(arr.as_ref().clone())),
                None => empty(),
            },
            "SolverOptions" => {
                let cancel = std::sync::atomic::AtomicBool::new(false);
                Ok(Value::Container(self.solver_options.to_struct(&cancel)?))
            }
            _ => Err(format!(
                "No appropriate method, property, or field '{name}' for class '{MODEL_CLASS}'."
            )),
        }
    }

    pub(crate) fn property_ci(&self, name: &str) -> Result<Value> {
        if let Some(&canonical) = MODEL_PROPERTIES
            .iter()
            .find(|p| p.eq_ignore_ascii_case(name))
        {
            self.property(canonical)
        } else {
            self.property(name)
        }
    }

    pub(crate) fn get_method(&self, e: &Engine, rest: &[Value]) -> Result<Value> {
        match rest {
            [] => {
                let mut fields = Vec::with_capacity(MODEL_PROPERTIES.len());
                for &prop in &MODEL_PROPERTIES {
                    let val = self.property(prop)?.array(&e.cancel)?;
                    fields.push((prop.to_owned(), Arc::new(val)));
                }
                let names = fields.iter().map(|(n, _)| n.clone()).collect();
                let slots = fields.into_iter().map(|(_, v)| v).collect();
                Ok(Value::Container(Array::Struct(StructArray::from_slots(
                    1, 1, names, slots, &e.cancel,
                )?)))
            }
            [prop] => {
                let name = text_value(prop).ok_or("pde.PDEModel: property name must be text")?;
                self.property_ci(&name)
            }
            _ => Err("pde.PDEModel: get expects zero or one property name".into()),
        }
    }

    pub(crate) fn body(&self) -> String {
        let geom = if self.geometry.is_some() {
            format!("[1x1 {GEOMETRY_CLASS}]")
        } else {
            "[]".into()
        };
        let slot = |opt: &Option<Arc<Array>>, label: &str| -> String {
            if opt.is_some() {
                label.into()
            } else {
                "[]".into()
            }
        };
        format!(
            "  PDEModel with properties:\n\n\
             {:>24}: {}\n\
             {:>24}: {}\n\
             {:>24}: {}\n\
             {:>24}: {}\n\
             {:>24}: {}\n\
             {:>24}: {}\n\
             {:>24}: {}\n\
             {:>24}: [1x1 PDEToolboxSolverOptions]\n",
            "PDESystemSize",
            self.system_size,
            "IsTimeDependent",
            u8::from(self.is_time_dependent),
            "Geometry",
            geom,
            "Mesh",
            slot(&self.mesh, "[1x1 FEMesh]"),
            "EquationCoefficients",
            slot(
                &self.equation_coefficients,
                "[1x1 CoefficientAssignmentRecords]"
            ),
            "BoundaryConditions",
            slot(&self.boundary_conditions, "[1x1 BoundaryConditionRecords]"),
            "InitialConditions",
            slot(&self.initial_conditions, "[1x1 InitialConditionsRecords]"),
            "SolverOptions",
        )
    }
}

/// Internal hook for future `generateMesh` (8a4) and integration tests to attach a mesh slot.
#[doc(hidden)]
pub fn attach_mesh_for_test(e: &mut Engine, var_name: &str, mesh: Array) -> Result<()> {
    let id = objects::pde_model_var_id(e, var_name)
        .ok_or_else(|| format!("'{var_name}' is not a live pde.PDEModel handle"))?;
    let slot = Arc::new(mesh);
    objects::pde_model_update(e, id, |m| {
        m.mesh = Some(slot);
        Ok(())
    })
}

/// Attach a supplied triangulation to a model for solver integration tests.
#[doc(hidden)]
pub fn attach_tri_mesh_for_test(
    e: &mut Engine,
    var_name: &str,
    tri: super::TriMesh,
    opts: super::MeshOpts,
) -> Result<()> {
    let geometry = objects::pde_model_var_id(e, var_name)
        .and_then(|id| objects::pde_model_get(e, id))
        .and_then(|model| model.geometry.as_ref())
        .ok_or_else(|| format!("'{var_name}' must be a PDE model with geometry"))?;
    let mesh = super::pde_mesh::FeMesh::from_tri_mesh(&geometry.0.geom, tri, opts)?;
    let mesh = objects::pde_mesh_new(e, Arc::new(mesh))?.array(&e.cancel)?;
    attach_mesh_for_test(e, var_name, mesh)
}

// ---------------------------------------------------------------------------
// Builtins: createpde, geometryFromEdges, pdegplot

fn createpde(e: &mut Engine, args: &[Value]) -> Result<Value> {
    let n = match args {
        [] => 1usize,
        [v] => {
            let m = numeric_matrix(v, "createpde: N")?;
            if m.len() != 1 {
                return Err("createpde: N must be a positive integer scalar".into());
            }
            let x = m.data()[0];
            if x < 1.0 || x.fract() != 0.0 || x > MAX_SYSTEM_SIZE as f64 {
                return Err(format!(
                    "createpde: N must be an integer between 1 and {MAX_SYSTEM_SIZE}"
                ));
            }
            x as usize
        }
        _ => return Err("createpde: expected createpde() or createpde(N)".into()),
    };
    objects::pde_model_new(e, PdeModel::new(n))
}

fn validate_dl(dl: &Matrix, who: &str) -> Result<Geometry2> {
    if dl.is_nd() || dl.cols() == 0 || dl.rows() < 7 || dl.rows() > 12 {
        return Err(format!(
            "{who}: dl must be a 2-D decomposed geometry matrix with 7 to 12 rows and at least 1 column"
        ));
    }
    Geometry2::from_dl(dl).map_err(|msg| format!("{who}: {msg}"))
}

fn geometry_from_edges(e: &mut Engine, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err("geometryFromEdges: expected geometryFromEdges(model, dl)".into());
    }
    let model_id = objects::pde_model_id(e, &args[0], "geometryFromEdges")?;
    if let Some(model) = objects::pde_model_get(e, model_id)
        && model.mesh.is_some()
    {
        return Err(
            "geometryFromEdges: cannot change geometry on a model that already has a mesh; create a new PDEModel using createpde"
                .into(),
        );
    }
    let dl = numeric_matrix(&args[1], "geometryFromEdges: dl")?;
    let geom = validate_dl(&dl, "geometryFromEdges")?;
    let analytic = Arc::new(AnalyticGeometry::from_geometry2(geom));
    let geom_val = objects::pde_geometry_new(e, analytic.clone())?;
    let geom_arr = Arc::new(geom_val.clone().array(&e.cancel)?);
    objects::pde_model_update(e, model_id, |m| {
        let nf = analytic.geom.num_faces;
        m.geometry = Some((analytic, geom_arr));
        if !m.coeff_assignments.is_empty() {
            m.is_time_dependent =
                super::pde_solve::recompute_is_time_dependent(&m.coeff_assignments, Some(nf));
        }
        Ok(())
    })?;
    Ok(geom_val)
}

#[derive(Default)]
struct PlotOptions {
    edge_labels: bool,
    face_labels: bool,
    vertex_labels: bool,
}

fn parse_on_off(v: &Value, name: &str) -> Result<bool> {
    if let Some(s) = text_value(v) {
        if s.eq_ignore_ascii_case("on") {
            return Ok(true);
        }
        if s.eq_ignore_ascii_case("off") {
            return Ok(false);
        }
        return Err(format!(
            "pdegplot: '{name}' must be 'on', 'off', true, or false"
        ));
    }
    match v {
        Value::Logical(l) | Value::Container(Array::Logical(l)) if l.len() == 1 => {
            Ok(l.data()[0] != 0)
        }
        Value::Matrix(m) | Value::Container(Array::Numeric(m))
            if m.len() == 1 && (m.data()[0] == 0.0 || m.data()[0] == 1.0) =>
        {
            Ok(m.data()[0] != 0.0)
        }
        _ => Err(format!(
            "pdegplot: '{name}' must be 'on', 'off', true, or false"
        )),
    }
}

fn parse_plot_options(args: &[Value]) -> Result<PlotOptions> {
    if !args.len().is_multiple_of(2) {
        return Err("pdegplot: options must be name-value pairs".into());
    }
    let mut opts = PlotOptions::default();
    for pair in args.as_chunks::<2>().0 {
        let name = text_value(&pair[0])
            .ok_or("pdegplot: option names must be character vectors or strings")?;
        if name.eq_ignore_ascii_case("EdgeLabels") {
            opts.edge_labels = parse_on_off(&pair[1], "EdgeLabels")?;
        } else if name.eq_ignore_ascii_case("FaceLabels") {
            opts.face_labels = parse_on_off(&pair[1], "FaceLabels")?;
        } else if name.eq_ignore_ascii_case("VertexLabels") {
            opts.vertex_labels = parse_on_off(&pair[1], "VertexLabels")?;
        } else {
            return Err(format!(
                "pdegplot: unknown option '{name}' (expected 'EdgeLabels', 'FaceLabels', or 'VertexLabels')"
            ));
        }
    }
    Ok(opts)
}

fn extract_geometry(e: &Engine, v: &Value) -> Result<Arc<AnalyticGeometry>> {
    if let Some(g) = objects::pde_geometry_value(e, v) {
        return Ok(g);
    }
    if objects::is_pde_model(e, v) {
        let id = objects::pde_model_id(e, v, "pdegplot")?;
        let model =
            objects::pde_model_get(e, id).ok_or("pdegplot: invalid or deleted pde.PDEModel")?;
        return model
            .geometry
            .as_ref()
            .map(|(g, _)| g.clone())
            .ok_or_else(|| "pdegplot: model has no geometry; call geometryFromEdges first".into());
    }
    let dl = numeric_matrix(
        v,
        "pdegplot: first argument (expected PDEModel, AnalyticGeometry, or dl matrix)",
    )?;
    let geom = validate_dl(&dl, "pdegplot")?;
    Ok(Arc::new(AnalyticGeometry::from_geometry2(geom)))
}

fn sample_edge(edge: &GEdge) -> (Vec<f64>, Vec<f64>) {
    match edge.kind {
        GKind::Line => (
            vec![edge.start[0], edge.end[0]],
            vec![edge.start[1], edge.end[1]],
        ),
        GKind::Arc { .. } | GKind::EllipseArc { .. } => {
            let delta = (edge.t1 - edge.t0).abs();
            let steps = ((delta / FRAC_PI_2) * ARC_STEPS_PER_QUARTER)
                .ceil()
                .clamp(2.0, MAX_ARC_STEPS as f64) as usize;
            let mut xs = Vec::with_capacity(steps + 1);
            let mut ys = Vec::with_capacity(steps + 1);
            for k in 0..=steps {
                let s = k as f64 / steps as f64;
                let p = edge.point_at(s);
                xs.push(p[0]);
                ys.push(p[1]);
            }
            (xs, ys)
        }
    }
}

fn pdegplot(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    if args.is_empty() {
        return Err("pdegplot: expected pdegplot(model | g | dl, ...)".into());
    }
    let geom = extract_geometry(e, &args[0])?;
    let opts = parse_plot_options(&args[1..])?;

    let old = (e.current_figure, e.figure.clone(), e.other_figures.clone());
    let res = draw_geometry(e, &geom, &opts, out);
    if res.is_err() {
        (e.current_figure, e.figure, e.other_figures) = old;
    }
    res
}

fn draw_geometry(
    e: &mut Engine,
    geom: &AnalyticGeometry,
    opts: &PlotOptions,
    out: &mut Execution,
) -> Result<Value> {
    let ax = e.call("gca", vec![], out)?;
    let next = e.call("get", vec![ax.clone(), Value::Text("NextPlot".into())], out)?;
    let held = text_value(&next).as_deref() == Some("add");

    let used_series = e.figure.as_ref().map_or(0, |f| {
        if held {
            f.series_count()
        } else {
            f.series_count().saturating_sub(f.axes.series.len())
        }
    });
    let free_series = crate::plot::MAX_SERIES.saturating_sub(used_series);
    let num_edges = geom.geom.edges.len();
    if num_edges > 0 && free_series == 0 {
        return Err(format!(
            "pdegplot: target axes has no free series slots (maximum {} series); clear the axes or turn hold off",
            crate::plot::MAX_SERIES
        ));
    }

    let edge_color = Value::Matrix(Matrix::from_column_vec(1, 3, vec![0.0, 0.4470, 0.7410])?);
    let per_edge = num_edges <= free_series;
    let mut handles: Vec<Arc<GraphicsHandle>> =
        Vec::with_capacity(if per_edge { num_edges } else { 1 });
    let mut first = true;

    let draw_result = (|| -> Result<()> {
        if per_edge {
            for edge in &geom.geom.edges {
                e.check_cancel()?;
                let (xs, ys) = sample_edge(edge);
                let x_val = Value::Matrix(Matrix::from_column_vec(xs.len(), 1, xs)?);
                let y_val = Value::Matrix(Matrix::from_column_vec(ys.len(), 1, ys)?);
                let h = e.call(
                    "plot",
                    vec![
                        ax.clone(),
                        x_val,
                        y_val,
                        Value::Text("Color".into()),
                        edge_color.clone(),
                    ],
                    out,
                )?;
                if let Value::Container(Array::Graphics(arr)) = h {
                    handles.push(Arc::new(arr.scalar_handle()?.clone()));
                }
                if first {
                    first = false;
                    e.call("hold", vec![ax.clone(), Value::Text("on".into())], out)?;
                }
            }
        } else {
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for (i, edge) in geom.geom.edges.iter().enumerate() {
                e.check_cancel()?;
                if i > 0 {
                    xs.push(f64::NAN);
                    ys.push(f64::NAN);
                }
                let (ex, ey) = sample_edge(edge);
                xs.extend(ex);
                ys.extend(ey);
            }
            let x_val = Value::Matrix(Matrix::from_column_vec(xs.len(), 1, xs)?);
            let y_val = Value::Matrix(Matrix::from_column_vec(ys.len(), 1, ys)?);
            let h = e.call(
                "plot",
                vec![
                    ax.clone(),
                    x_val,
                    y_val,
                    Value::Text("Color".into()),
                    edge_color.clone(),
                ],
                out,
            )?;
            if let Value::Container(Array::Graphics(arr)) = h {
                handles.push(Arc::new(arr.scalar_handle()?.clone()));
            }
            e.call("hold", vec![ax.clone(), Value::Text("on".into())], out)?;
        }

        if opts.edge_labels {
            for (i, edge) in geom.geom.edges.iter().enumerate() {
                e.check_cancel()?;
                let p = edge.point_at(0.5);
                e.call(
                    "text",
                    vec![
                        ax.clone(),
                        Value::Matrix(Matrix::scalar(p[0])),
                        Value::Matrix(Matrix::scalar(p[1])),
                        Value::Text(format!("E{}", i + 1)),
                    ],
                    out,
                )?;
            }
        }

        if opts.face_labels {
            for f in 1..=geom.geom.num_faces {
                e.check_cancel()?;
                let p = face_interior_point(&geom.geom, f);
                e.call(
                    "text",
                    vec![
                        ax.clone(),
                        Value::Matrix(Matrix::scalar(p[0])),
                        Value::Matrix(Matrix::scalar(p[1])),
                        Value::Text(format!("F{f}")),
                    ],
                    out,
                )?;
            }
        }

        if opts.vertex_labels {
            for (i, v) in geom.vertices.iter().enumerate() {
                e.check_cancel()?;
                e.call(
                    "text",
                    vec![
                        ax.clone(),
                        Value::Matrix(Matrix::scalar(v[0])),
                        Value::Matrix(Matrix::scalar(v[1])),
                        Value::Text(format!("V{}", i + 1)),
                    ],
                    out,
                )?;
            }
        }

        e.call("axes", vec![ax.clone()], out)?;
        e.call("axis", vec![Value::Text("equal".into())], out)?;
        Ok(())
    })();

    let restore = e.call(
        "hold",
        vec![ax, Value::Text(if held { "on" } else { "off" }.into())],
        out,
    );
    draw_result?;
    restore?;

    let arr = GraphicsArray::from_shared(handles.len(), 1, handles, &e.cancel)?;
    Ok(Value::Container(Array::Graphics(arr)))
}

// ---------------------------------------------------------------------------
// Guaranteed interior point of a face (scanline + max distance to boundary)

fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn face_boundary_segments(geom: &Geometry2, face: usize) -> Vec<([f64; 2], [f64; 2])> {
    let mut segs = Vec::new();
    for edge in &geom.edges {
        let forward = if edge.left == face {
            true
        } else if edge.right == face {
            false
        } else {
            continue;
        };
        let steps = match edge.kind {
            GKind::Line => 1usize,
            GKind::Arc { .. } | GKind::EllipseArc { .. } => {
                let delta = (edge.t1 - edge.t0).abs();
                ((delta / FRAC_PI_2) * 32.0).ceil().clamp(4.0, 512.0) as usize
            }
        };
        for k in 0..steps {
            let s0 = k as f64 / steps as f64;
            let s1 = (k + 1) as f64 / steps as f64;
            let (p0, p1) = if forward {
                (edge.point_at(s0), edge.point_at(s1))
            } else {
                (edge.point_at(1.0 - s0), edge.point_at(1.0 - s1))
            };
            segs.push((p0, p1));
        }
    }
    segs
}

fn winding_inside(segs: &[([f64; 2], [f64; 2])], p: [f64; 2]) -> bool {
    let mut winding = 0i32;
    for &(a, b) in segs {
        if a[1] <= p[1] {
            if b[1] > p[1] && orient(a, b, p) > 0.0 {
                winding += 1;
            }
        } else if b[1] <= p[1] && orient(a, b, p) < 0.0 {
            winding -= 1;
        }
    }
    winding > 0
}

fn point_segment_dist_sq(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        let ex = p[0] - a[0];
        let ey = p[1] - a[1];
        return ex * ex + ey * ey;
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0);
    let qx = a[0] + t * dx;
    let qy = a[1] + t * dy;
    let ex = p[0] - qx;
    let ey = p[1] - qy;
    ex * ex + ey * ey
}

fn min_dist_sq(segs: &[([f64; 2], [f64; 2])], p: [f64; 2]) -> f64 {
    segs.iter()
        .map(|&(a, b)| point_segment_dist_sq(p, a, b))
        .fold(f64::INFINITY, f64::min)
}

fn unique_midpoints(mut coords: Vec<f64>, tol: f64, limit: usize) -> Vec<f64> {
    coords.sort_by(f64::total_cmp);
    let mut uniq: Vec<f64> = Vec::new();
    for c in coords {
        if uniq.last().is_none_or(|&prev| (c - prev).abs() > tol) {
            uniq.push(c);
        }
    }
    if uniq.len() < 2 {
        return Vec::new();
    }
    let total = uniq.len() - 1;
    let stride = (total / limit).max(1);
    let mut mids = Vec::new();
    for i in (0..total).step_by(stride) {
        mids.push(0.5 * (uniq[i] + uniq[i + 1]));
    }
    mids
}

fn face_interior_point(geom: &Geometry2, face: usize) -> [f64; 2] {
    let segs = face_boundary_segments(geom, face);
    if segs.is_empty() {
        return [0.0, 0.0];
    }
    let (mut xmin, mut xmax) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut ymin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut vx = Vec::with_capacity(segs.len());
    let mut vy = Vec::with_capacity(segs.len());
    for &(a, _) in &segs {
        xmin = xmin.min(a[0]);
        xmax = xmax.max(a[0]);
        ymin = ymin.min(a[1]);
        ymax = ymax.max(a[1]);
        vx.push(a[0]);
        vy.push(a[1]);
    }
    let span = (xmax - xmin).max(ymax - ymin).max(f64::MIN_POSITIVE);
    let tol = 1e-12 * span;

    let mut best: Option<([f64; 2], f64)> = None;
    let mut consider = |p: [f64; 2]| {
        if winding_inside(&segs, p) {
            let d2 = min_dist_sq(&segs, p);
            if best.is_none_or(|(_, bd2)| d2 > bd2) {
                best = Some((p, d2));
            }
        }
    };

    // Horizontal scanlines (uniform + midpoints between vertex y-levels).
    let n_uniform = 31usize;
    let mut ys: Vec<f64> = (0..n_uniform)
        .map(|k| ymin + (k as f64 + 0.5) / n_uniform as f64 * (ymax - ymin))
        .collect();
    ys.extend(unique_midpoints(vy, tol, 64));
    let mut xs_buf = Vec::new();
    for y in ys {
        xs_buf.clear();
        for &(a, b) in &segs {
            if (a[1] <= y && b[1] > y) || (b[1] <= y && a[1] > y) {
                let t = (y - a[1]) / (b[1] - a[1]);
                xs_buf.push(a[0] + t * (b[0] - a[0]));
            }
        }
        xs_buf.sort_by(f64::total_cmp);
        for pair in xs_buf.as_chunks::<2>().0 {
            if pair[1] > pair[0] {
                consider([0.5 * (pair[0] + pair[1]), y]);
            }
        }
    }

    // Vertical scanlines (uniform + midpoints between vertex x-levels).
    let mut xs: Vec<f64> = (0..n_uniform)
        .map(|k| xmin + (k as f64 + 0.5) / n_uniform as f64 * (xmax - xmin))
        .collect();
    xs.extend(unique_midpoints(vx, tol, 64));
    let mut ys_buf = Vec::new();
    for x in xs {
        ys_buf.clear();
        for &(a, b) in &segs {
            if (a[0] <= x && b[0] > x) || (b[0] <= x && a[0] > x) {
                let t = (x - a[0]) / (b[0] - a[0]);
                ys_buf.push(a[1] + t * (b[1] - a[1]));
            }
        }
        ys_buf.sort_by(f64::total_cmp);
        for pair in ys_buf.as_chunks::<2>().0 {
            if pair[1] > pair[0] {
                consider([x, 0.5 * (pair[0] + pair[1])]);
            }
        }
    }

    // Local grid refinement around the best interior candidate.
    if let Some((mut center, mut best_d2)) = best {
        let mut step = best_d2.sqrt() * 0.5;
        for _ in 0..3 {
            if step.is_nan() || step <= 0.0 {
                break;
            }
            let base = center;
            for ix in -2..=2 {
                for iy in -2..=2 {
                    if ix == 0 && iy == 0 {
                        continue;
                    }
                    let cand = [
                        base[0] + f64::from(ix) * step,
                        base[1] + f64::from(iy) * step,
                    ];
                    if winding_inside(&segs, cand) {
                        let d2 = min_dist_sq(&segs, cand);
                        if d2 > best_d2 {
                            center = cand;
                            best_d2 = d2;
                        }
                    }
                }
            }
            step *= 0.5;
        }
        return center;
    }

    [0.5 * (xmin + xmax), 0.5 * (ymin + ymax)]
}

// ---------------------------------------------------------------------------
// Value helpers

fn numeric_matrix(v: &Value, label: &str) -> Result<Matrix> {
    let m = match v {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => m.clone(),
        Value::Logical(l) | Value::Container(Array::Logical(l)) => {
            let data = l.data().iter().map(|&b| f64::from(b)).collect();
            Matrix::from_column_vec(l.rows(), l.cols(), data)?
        }
        Value::Container(Array::Typed(a)) => a.to_matrix()?,
        _ => return Err(format!("{label} must be a real numeric matrix")),
    };
    if m.is_nd() || m.data().iter().any(|x| !x.is_finite()) {
        return Err(format!("{label} must be a finite real 2-D matrix"));
    }
    Ok(m)
}

fn text_value(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            s.data()[0].as_deref().map(str::to_owned)
        }
        _ => None,
    }
}
