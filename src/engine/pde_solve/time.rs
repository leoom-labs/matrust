//! 2-D time-dependent PDE initial conditions (`setInitialConditions`),
//! parabolic & hyperbolic time integration (`solvepde(model, tlist)` via `ode15s`),
//! and results object (`pde.TimeDependentResults`).
use super::assemble::{
    BoundaryFacet, EdgeQuadPoint, assemble_kmf_sys, assemble_load_sys, assemble_mass_sys,
    assemble_neumann_sys, collect_domain_quad_points, collect_edge_quad_points, element_faces,
    extract_boundary_facets, partition_matrix, sparse_matvec_add, sparse_scatter_dense,
};
use super::postproc::compute_nodal_gradients;
use super::types::{
    BcAssignment, BcKind, CoeffAssignment, CoeffC, CoeffVal, IcAssignment, IcValue, SolverOptions,
    expand_vec_coeff, ic_records_array, numeric_matrix, text_value,
};
use super::{
    EVAL_CHUNK, build_location_struct, check_coeff_workspace, eval_c_sys_with_state,
    eval_mat_sys_with_state, eval_neumann_on_edge_pts, eval_vec_sys_with_state,
    evaluate_dirichlet_sys, format_dims, parse_region_ids, resolve_edge_comp_bcs,
};
use crate::engine::ode_more::ivp::{self, JacKind, MassKind, Model, Opts, OutAt, Setup};
use crate::engine::{Engine, Execution, Value, objects, pde_geometry::Geometry2, pde_mesh::FeMesh};
use crate::sparse::SparseMatrix;
use crate::{Array, FunctionHandle, Matrix, Result, StructArray};
use std::sync::Arc;

pub(crate) const TIME_RESULTS_CLASS: &str = "pde.TimeDependentResults";
pub(crate) const TIME_RESULTS_PROPERTIES: [&str; 5] = [
    "NodalSolution",
    "SolutionTimes",
    "XGradients",
    "YGradients",
    "Mesh",
];

/// Maximum ODE state dimension (`n_free` for parabolic, `2 * n_free` for hyperbolic)
/// because `ode15s` uses a dense `n_sys x n_sys` LU factorization.
const MAX_TIME_DOFS: usize = 4096;

// ---------------------------------------------------------------------------
// pde.TimeDependentResults (value semantics)

#[derive(Clone, Debug)]
pub(crate) struct TimeDependentResults {
    pub(crate) nodal_solution: Matrix,
    pub(crate) solution_times: Matrix,
    pub(crate) x_gradients: Matrix,
    pub(crate) y_gradients: Matrix,
    pub(crate) mesh: Arc<FeMesh>,
    pub(crate) mesh_arr: Arc<Array>,
}

impl PartialEq for TimeDependentResults {
    fn eq(&self, other: &Self) -> bool {
        self.nodal_solution.dims() == other.nodal_solution.dims()
            && self.nodal_solution.data() == other.nodal_solution.data()
            && self.solution_times.rows() == other.solution_times.rows()
            && self.solution_times.cols() == other.solution_times.cols()
            && self.solution_times.data() == other.solution_times.data()
            && self.mesh == other.mesh
    }
}

impl TimeDependentResults {
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + (self.nodal_solution.len()
                + self.solution_times.len()
                + self.x_gradients.len()
                + self.y_gradients.len())
                * std::mem::size_of::<f64>()
            + self.mesh_arr.retained_bytes()
    }

    pub(crate) fn push_children<'a>(&'a self, pending: &mut Vec<&'a Array>) {
        pending.push(self.mesh_arr.as_ref());
    }

    pub(crate) fn property(&self, name: &str) -> Result<Value> {
        match name {
            "NodalSolution" => Ok(Value::Matrix(self.nodal_solution.clone())),
            "SolutionTimes" => Ok(Value::Matrix(self.solution_times.clone())),
            "XGradients" => Ok(Value::Matrix(self.x_gradients.clone())),
            "YGradients" => Ok(Value::Matrix(self.y_gradients.clone())),
            "Mesh" => Ok(Value::Container(self.mesh_arr.as_ref().clone())),
            _ => Err(format!(
                "No appropriate method, property, or field '{name}' for class '{TIME_RESULTS_CLASS}'."
            )),
        }
    }

    pub(crate) fn property_ci(&self, name: &str) -> Result<Value> {
        if let Some(&canonical) = TIME_RESULTS_PROPERTIES
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
                let mut fields = Vec::with_capacity(TIME_RESULTS_PROPERTIES.len());
                for &prop in &TIME_RESULTS_PROPERTIES {
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
                let name = text_value(prop)
                    .ok_or("pde.TimeDependentResults: property name must be text")?;
                self.property_ci(&name)
            }
            _ => Err("pde.TimeDependentResults: get expects zero or one property name".into()),
        }
    }

    pub(crate) fn body(&self) -> String {
        format!(
            "  TimeDependentResults with properties:\n\n\
             {:>18}: [{} double]\n\
             {:>18}: [{}x{} double]\n\
             {:>18}: [{} double]\n\
             {:>18}: [{} double]\n\
             {:>18}: [1x1 FEMesh]\n",
            "NodalSolution",
            format_dims(&self.nodal_solution),
            "SolutionTimes",
            self.solution_times.rows(),
            self.solution_times.cols(),
            "XGradients",
            format_dims(&self.x_gradients),
            "YGradients",
            format_dims(&self.y_gradients),
            "Mesh",
        )
    }
}

// ---------------------------------------------------------------------------
// setInitialConditions

fn parse_ic_value(
    e: &Engine,
    v: &Value,
    n_comp: usize,
    mesh_node_count: Option<usize>,
    label: &str,
) -> Result<IcValue> {
    if let Some(res) = objects::pde_stationary_results_value(e, v) {
        let ns = &res.nodal_solution;
        if ns.cols() != n_comp || ns.rows() == 0 {
            return Err(format!(
                "setInitialConditions: {label} results must have an N-by-{n_comp} NodalSolution"
            ));
        }
        if let Some(n_mesh) = mesh_node_count
            && ns.rows() != n_mesh
        {
            return Err(format!(
                "setInitialConditions: results mesh has {} nodes, but model mesh has {n_mesh} nodes",
                ns.rows()
            ));
        }
        return Ok(IcValue::Nodal {
            data: Arc::new(ns.data().to_vec()),
            n_comp,
        });
    }
    if let Value::Container(Array::Handle(h)) = v {
        return Ok(IcValue::Handle(h.clone()));
    }
    let m = numeric_matrix(v, &format!("setInitialConditions: {label}"))?;
    if m.len() == 1
        || (m.cols() == 1 && m.rows() == n_comp)
        || (m.rows() == 1 && m.cols() == n_comp)
    {
        return Ok(IcValue::Const(m.data().to_vec()));
    }
    Err(format!(
        "setInitialConditions: {label} must be a real finite scalar, length-{n_comp} vector, function handle, or pde.StationaryResults"
    ))
}

pub(crate) fn set_initial_conditions(e: &mut Engine, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 5 {
        return Err(
            "setInitialConditions: expected setInitialConditions(model, u0[, ut0][, 'Face', ids])"
                .into(),
        );
    }
    let model_id = objects::pde_model_id(e, &args[0], "setInitialConditions")?;
    let (system_size, num_faces, mesh_node_count) = {
        let model = objects::pde_model_get(e, model_id)
            .ok_or("setInitialConditions: invalid or deleted pde.PDEModel")?;
        let n_nodes = model.mesh.as_ref().and_then(|arr| {
            let v = Value::Container(arr.as_ref().clone());
            objects::pde_mesh_value(e, &v).map(|m| m.nodes.len())
        });
        (
            model.system_size,
            model.geometry.as_ref().map(|(g, _)| g.geom.num_faces),
            n_nodes,
        )
    };

    let u0 = parse_ic_value(e, &args[1], system_size, mesh_node_count, "u0")?;
    let (ut0, faces) = match &args[2..] {
        [] => (None, None),
        [a] => (
            Some(parse_ic_value(e, a, system_size, mesh_node_count, "ut0")?),
            None,
        ),
        [a, b] => {
            let key = text_value(a)
                .ok_or("setInitialConditions: expected 'Face' region name-value pair after u0")?;
            if !key.eq_ignore_ascii_case("Face") {
                return Err(format!(
                    "setInitialConditions: unknown region parameter '{key}' (expected 'Face')"
                ));
            }
            let ids = parse_region_ids(b, num_faces, "setInitialConditions: 'Face'")?;
            (None, Some(ids))
        }
        [a, b, c] => {
            let ut = parse_ic_value(e, a, system_size, mesh_node_count, "ut0")?;
            let key = text_value(b)
                .ok_or("setInitialConditions: expected 'Face' region name-value pair after ut0")?;
            if !key.eq_ignore_ascii_case("Face") {
                return Err(format!(
                    "setInitialConditions: unknown region parameter '{key}' (expected 'Face')"
                ));
            }
            let ids = parse_region_ids(c, num_faces, "setInitialConditions: 'Face'")?;
            (Some(ut), Some(ids))
        }
        _ => unreachable!(),
    };

    let assignment = IcAssignment { faces, u0, ut0 };
    let ret_struct = assignment.to_struct(&e.cancel)?;
    let cancel = e.cancel.clone();
    objects::pde_model_update(e, model_id, |model| {
        model.ic_assignments.push(assignment);
        let rec = ic_records_array(&model.ic_assignments, &cancel)?;
        model.initial_conditions = Some(Arc::new(rec));
        Ok(())
    })?;

    Ok(Value::Container(ret_struct))
}

// ---------------------------------------------------------------------------
// Initial condition evaluation at mesh nodes (point-major: pt * n_comp + comp)

#[allow(clippy::too_many_arguments)]
fn invoke_ic_handle(
    e: &mut Engine,
    out: &mut Execution,
    h: &FunctionHandle,
    n_comp: usize,
    xs: &[f64],
    ys: &[f64],
    subs: &[f64],
    label: &str,
) -> Result<Vec<f64>> {
    let n = xs.len();
    let mut result = Vec::with_capacity(n * n_comp);
    let mut start = 0;
    while start < n {
        let end = (start + EVAL_CHUNK).min(n);
        let nr = end - start;
        let loc = build_location_struct(
            &xs[start..end],
            &ys[start..end],
            &subs[start..end],
            None,
            &e.cancel,
        )?;
        e.check_cancel()?;
        let evaluated = e.evaluated_elements;
        let res = e.call_handle(h, vec![loc], 1, None, out);
        e.evaluated_elements = evaluated;
        let vals = res?;
        let v = vals
            .into_iter()
            .next()
            .ok_or_else(|| format!("solvepde: {label} function handle returned no output"))?;
        let m = numeric_matrix(&v, &format!("solvepde: {label} function handle"))?;
        let data = m.data();
        if m.rows() == n_comp && m.cols() == nr {
            result.extend_from_slice(data);
        } else if m.rows() == 1 && m.cols() == nr {
            for &val in data {
                for _ in 0..n_comp {
                    result.push(val);
                }
            }
        } else if m.rows() == 1 && m.cols() == 1 {
            result.resize(result.len() + nr * n_comp, data[0]);
        } else if m.rows() == n_comp && m.cols() == 1 {
            for _ in 0..nr {
                result.extend_from_slice(data);
            }
        } else if n_comp == 1 && m.rows() == nr && m.cols() == 1 {
            result.extend_from_slice(data);
        } else {
            return Err(format!(
                "solvepde: {label} function handle must return {n_comp}-by-{nr} (got {}x{})",
                m.rows(),
                m.cols()
            ));
        }
        start = end;
    }
    Ok(result)
}

fn eval_ic_value_on_nodes(
    e: &mut Engine,
    out: &mut Execution,
    ic: &IcValue,
    n_comp: usize,
    mesh: &FeMesh,
    nodes: &[(usize, usize)],
    label: &str,
) -> Result<Vec<f64>> {
    match ic {
        IcValue::Const(v) => {
            let expanded = expand_vec_coeff(v, n_comp, label)?;
            let mut out_v = Vec::with_capacity(nodes.len() * n_comp);
            for _ in 0..nodes.len() {
                out_v.extend_from_slice(&expanded);
            }
            Ok(out_v)
        }
        IcValue::Handle(h) => {
            let xs: Vec<f64> = nodes.iter().map(|&(ni, _)| mesh.nodes[ni][0]).collect();
            let ys: Vec<f64> = nodes.iter().map(|&(ni, _)| mesh.nodes[ni][1]).collect();
            let subs: Vec<f64> = nodes.iter().map(|&(_, fid)| fid as f64).collect();
            invoke_ic_handle(e, out, h, n_comp, &xs, &ys, &subs, label)
        }
        IcValue::Nodal { data, n_comp: nc } => {
            let n_nodes = mesh.nodes.len();
            if *nc != n_comp || data.len() != n_nodes * n_comp {
                return Err(format!(
                    "solvepde: initial condition results size does not match model mesh ({n_nodes} nodes, {n_comp} components)"
                ));
            }
            let mut out_v = Vec::with_capacity(nodes.len() * n_comp);
            for &(ni, _) in nodes {
                for comp in 0..n_comp {
                    out_v.push(data[comp * n_nodes + ni]);
                }
            }
            Ok(out_v)
        }
    }
}

fn evaluate_initial_conditions(
    e: &mut Engine,
    out: &mut Execution,
    mesh: &FeMesh,
    n_comp: usize,
    num_faces: usize,
    ic_assignments: &[IcAssignment],
) -> Result<(Vec<f64>, Vec<f64>)> {
    let n_nodes = mesh.nodes.len();
    let n_dofs = n_nodes * n_comp;
    let mut u0 = vec![0.0; n_dofs];
    let mut ut0 = vec![0.0; n_dofs];
    let mut assigned = vec![false; n_nodes];

    let mut node_face = vec![1usize; n_nodes];
    for (fi, fnodes) in mesh.face_nodes.iter().enumerate() {
        let fid = fi + 1;
        for &n1 in fnodes {
            if n1 >= 1 && n1 <= n_nodes {
                node_face[n1 - 1] = fid;
            }
        }
    }

    for ic in ic_assignments {
        let target: Vec<(usize, usize)> = match &ic.faces {
            None => (0..n_nodes).map(|ni| (ni, node_face[ni])).collect(),
            Some(ids) => {
                let mut list = Vec::new();
                for &fid in ids {
                    if fid < 1 || fid > num_faces {
                        return Err(format!(
                            "solvepde: initial condition Face ID {fid} exceeds geometry NumFaces ({num_faces})"
                        ));
                    }
                    for &n1 in &mesh.face_nodes[fid - 1] {
                        list.push((n1 - 1, fid));
                    }
                }
                list
            }
        };
        if target.is_empty() {
            continue;
        }
        let u0_vals = eval_ic_value_on_nodes(
            e,
            out,
            &ic.u0,
            n_comp,
            mesh,
            &target,
            "initial condition u0",
        )?;
        for (k, &(ni, _)) in target.iter().enumerate() {
            for comp in 0..n_comp {
                u0[comp * n_nodes + ni] = u0_vals[k * n_comp + comp];
            }
            assigned[ni] = true;
        }
        if let Some(ut_ic) = &ic.ut0 {
            let ut0_vals = eval_ic_value_on_nodes(
                e,
                out,
                ut_ic,
                n_comp,
                mesh,
                &target,
                "initial condition ut0",
            )?;
            for (k, &(ni, _)) in target.iter().enumerate() {
                for comp in 0..n_comp {
                    ut0[comp * n_nodes + ni] = ut0_vals[k * n_comp + comp];
                }
            }
        } else {
            for &(ni, _) in &target {
                for comp in 0..n_comp {
                    ut0[comp * n_nodes + ni] = 0.0;
                }
            }
        }
    }

    if assigned.iter().any(|&a| !a) {
        return Err(
            "solvepde: Initial conditions must be specified for all faces of the geometry. Use setInitialConditions."
                .into(),
        );
    }
    Ok((u0, ut0))
}

// ---------------------------------------------------------------------------
// ODE Model adapter for ode15s

struct TimePdeModel<'a> {
    e: &'a mut Engine,
    out: &'a mut Execution,
    mesh: &'a FeMesh,
    geom: &'a Geometry2,
    n_comp: usize,
    is_hyperbolic: bool,
    n_free: usize,
    free_dofs: &'a [usize],
    dir_dofs: &'a [usize],
    k_ii: &'a SparseMatrix,
    k_id: &'a SparseMatrix,
    m_d_ii: Option<&'a SparseMatrix>,
    m_d_id: Option<&'a SparseMatrix>,
    m_m_id: Option<&'a SparseMatrix>,
    const_f_eff: Option<Vec<f64>>,
    base_f_i: Vec<f64>,
    f_time_dep: bool,
    dom_xs: Vec<Vec<f64>>,
    dom_ys: Vec<Vec<f64>>,
    dom_subs: Vec<Vec<f64>>,
    dom_groups: Vec<Vec<usize>>,
    n_dom: usize,
    coeff_assignments: &'a [CoeffAssignment],
    g_time_dep: bool,
    neumann_facets: &'a [BoundaryFacet],
    edge_pts: &'a [EdgeQuadPoint],
    facet_edges: &'a [usize],
    nq_edge: usize,
    edge_comp_bc: &'a [Vec<Option<usize>>],
    dir_time_dep: bool,
    bc_assignments: &'a [BcAssignment],
    const_u_d: Vec<f64>,
    t_lo: f64,
    t_hi: f64,
    /// Handle-valued m, d, a, c that take `state`, with their t0 values (checked every step).
    frozen: Vec<FrozenCoeff>,
    /// t0 values of the Neumann `q` at the edge points when some active `q` takes `state`.
    q_t0: Option<Vec<f64>>,
    /// Last time the frozen coefficients were checked (`NaN` before the first check).
    checked_t: f64,
}

/// Coefficient that the time integrator treats as constant.
#[derive(Clone, Copy, PartialEq)]
enum FrozenKind {
    M,
    D,
    A,
    C,
}

impl FrozenKind {
    fn error(self) -> String {
        match self {
            Self::M | Self::D => {
                "solvepde: time-dependent 'm' or 'd' coefficients are not supported yet".into()
            }
            Self::A => "solvepde: time-dependent 'a' coefficient is not supported yet".into(),
            Self::C => "solvepde: time-dependent 'c' coefficient is not supported yet".into(),
        }
    }
}

/// A handle-valued m, d, a, or c coefficient (one `CoeffAssignment`) and its values at t0.
struct FrozenCoeff {
    ca_idx: usize,
    kind: FrozenKind,
    t0_vals: Vec<f64>,
}

/// Whether a coefficient/BC handle can read `state` (and so `state.time`): anonymous
/// handles with a second parameter or `varargin`, and every named handle.
fn handle_takes_state(h: &FunctionHandle) -> bool {
    match h.anonymous_parts() {
        Some((f, _)) => f.params.len() >= 2 || f.params.iter().any(|p| p == "varargin"),
        None => true,
    }
}

fn coeff_takes_state(c: &CoeffVal) -> bool {
    matches!(c, CoeffVal::Handle(h) if handle_takes_state(h))
}

impl TimePdeModel<'_> {
    /// Re-evaluate the m, d, a, c, and Neumann q handles that take `state` at time `t`;
    /// any change from the t0 values is an error (the integrator uses the t0 matrices).
    fn check_frozen(&mut self, t: f64) -> Result<()> {
        if self.checked_t == t || (self.frozen.is_empty() && self.q_t0.is_none()) {
            return Ok(());
        }
        let nc = self.n_comp;
        for k in 0..self.frozen.len() {
            let (ca_idx, kind) = (self.frozen[k].ca_idx, self.frozen[k].kind);
            let ca = &self.coeff_assignments[ca_idx];
            let (xs, ys, subs) = (
                &self.dom_xs[ca_idx],
                &self.dom_ys[ca_idx],
                &self.dom_subs[ca_idx],
            );
            let vals = match kind {
                FrozenKind::C => {
                    eval_c_sys_with_state(self.e, self.out, &ca.c, nc, xs, ys, subs, t, None)?
                }
                FrozenKind::M | FrozenKind::D | FrozenKind::A => {
                    let (coeff, label) = match kind {
                        FrozenKind::M => (&ca.m, "coefficient 'm'"),
                        FrozenKind::D => (&ca.d, "coefficient 'd'"),
                        _ => (&ca.a, "coefficient 'a'"),
                    };
                    eval_mat_sys_with_state(
                        self.e, self.out, coeff, nc, xs, ys, subs, None, t, None, label,
                    )?
                }
            };
            if vals != self.frozen[k].t0_vals {
                return Err(kind.error());
            }
        }
        if let Some(q_t0) = &self.q_t0 {
            let (q_vals, _) = eval_neumann_on_edge_pts(
                self.e,
                self.out,
                nc,
                self.bc_assignments,
                self.edge_comp_bc,
                self.facet_edges,
                self.nq_edge,
                self.edge_pts,
                t,
                None,
                true,
            )?;
            if &q_vals != q_t0 {
                return Err(
                    "solvepde: time-dependent Neumann 'q' coefficient is not supported yet".into(),
                );
            }
        }
        self.checked_t = t;
        Ok(())
    }

    fn eval_u_d(&mut self, t: f64) -> Result<Vec<f64>> {
        if !self.dir_time_dep {
            return Ok(self.const_u_d.clone());
        }
        let (_, full) = evaluate_dirichlet_sys(
            self.e,
            self.out,
            self.mesh,
            self.geom,
            self.n_comp,
            self.bc_assignments,
            self.edge_comp_bc,
            t,
            false,
        )?;
        Ok(self.dir_dofs.iter().map(|&d| full[d]).collect())
    }

    fn eval_f_eff(&mut self, t: f64) -> Result<Vec<f64>> {
        if let Some(fe) = &self.const_f_eff {
            return Ok(fe.clone());
        }
        let nc = self.n_comp;
        let n_dofs = self.mesh.nodes.len() * nc;
        let mut f_eff = if !self.f_time_dep && !self.g_time_dep {
            self.base_f_i.clone()
        } else {
            let mut f_full = if self.f_time_dep {
                let mut f_vals = vec![0.0; self.n_dom * nc];
                for (ca_idx, indices) in self.dom_groups.iter().enumerate() {
                    if indices.is_empty() {
                        continue;
                    }
                    let fv = eval_vec_sys_with_state(
                        self.e,
                        self.out,
                        &self.coeff_assignments[ca_idx].f,
                        nc,
                        &self.dom_xs[ca_idx],
                        &self.dom_ys[ca_idx],
                        &self.dom_subs[ca_idx],
                        None,
                        t,
                        None,
                        "coefficient 'f'",
                    )?;
                    for (k, &q_idx) in indices.iter().enumerate() {
                        f_vals[q_idx * nc..(q_idx + 1) * nc]
                            .copy_from_slice(&fv[k * nc..(k + 1) * nc]);
                    }
                }
                assemble_load_sys(self.mesh, nc, &f_vals, &self.e.cancel)?
            } else {
                vec![0.0; n_dofs]
            };

            if self.g_time_dep && !self.neumann_facets.is_empty() {
                let q_zeros = vec![0.0; self.edge_pts.len() * nc * nc];
                let (_, g_vals) = eval_neumann_on_edge_pts(
                    self.e,
                    self.out,
                    nc,
                    self.bc_assignments,
                    self.edge_comp_bc,
                    self.facet_edges,
                    self.nq_edge,
                    self.edge_pts,
                    t,
                    None,
                    false,
                )?;
                let (_, g_vec) = assemble_neumann_sys(
                    self.mesh,
                    nc,
                    self.neumann_facets,
                    &q_zeros,
                    &g_vals,
                    &self.e.cancel,
                )?;
                for (fi, &gi) in f_full.iter_mut().zip(g_vec.iter()) {
                    *fi += gi;
                }
            }

            if self.f_time_dep && self.g_time_dep {
                self.free_dofs.iter().map(|&i| f_full[i]).collect()
            } else {
                self.free_dofs
                    .iter()
                    .enumerate()
                    .map(|(fi, &i)| self.base_f_i[fi] + f_full[i])
                    .collect()
            }
        };

        if !self.dir_dofs.is_empty() {
            let u_d = self.eval_u_d(t)?;
            sparse_matvec_add(self.k_id, &u_d, -1.0, &mut f_eff);

            if self.dir_time_dep {
                let need_du = self.m_d_id.is_some_and(|m| m.nnz() > 0);
                let need_d2u = self.is_hyperbolic && self.m_m_id.is_some_and(|m| m.nnz() > 0);
                if need_du || need_d2u {
                    let (du, d2u) = self.dirichlet_time_derivs(t, &u_d, need_du, need_d2u)?;
                    if let (Some(du), Some(m_d_id)) = (du, self.m_d_id) {
                        sparse_matvec_add(m_d_id, &du, -1.0, &mut f_eff);
                    }
                    if let (Some(d2u), Some(m_m_id)) = (d2u, self.m_m_id) {
                        sparse_matvec_add(m_m_id, &d2u, -1.0, &mut f_eff);
                    }
                }
            }
        }

        Ok(f_eff)
    }

    #[allow(clippy::type_complexity)]
    fn dirichlet_time_derivs(
        &mut self,
        t: f64,
        u_d: &[f64],
        need_du: bool,
        need_d2u: bool,
    ) -> Result<(Option<Vec<f64>>, Option<Vec<f64>>)> {
        let scale = 1.0 + t.abs();
        let base = if need_d2u { 1e-5 * scale } else { 1e-7 * scale };
        let lo = self.t_lo;
        let hi = self.t_hi;
        let back = (t - lo).max(0.0);
        let fwd = (hi - t).max(0.0);
        let steps = if need_d2u { 2.0 } else { 1.0 };
        let zeros = || vec![0.0; u_d.len()];

        let mode = if back >= base && fwd >= base {
            0
        } else if fwd >= back && fwd > 0.0 {
            1
        } else if back > 0.0 {
            2
        } else {
            return Ok((need_du.then(zeros), need_d2u.then(zeros)));
        };
        let dt = if mode == 0 {
            base
        } else if mode == 1 {
            base.min(fwd / steps)
        } else {
            base.min(back / steps)
        };
        if !(dt.is_finite() && dt > 0.0) {
            return Ok((need_du.then(zeros), need_d2u.then(zeros)));
        }
        let clamp = |s: f64| s.clamp(lo, hi);

        if mode == 0 {
            let u_plus = self.eval_u_d(clamp(t + dt))?;
            let u_minus = self.eval_u_d(clamp(t - dt))?;
            let du = need_du.then(|| {
                u_plus
                    .iter()
                    .zip(u_minus.iter())
                    .map(|(p, m)| (p - m) / (2.0 * dt))
                    .collect()
            });
            let d2u = need_d2u.then(|| {
                let inv_dt2 = 1.0 / (dt * dt);
                u_plus
                    .iter()
                    .zip(u_d.iter())
                    .zip(u_minus.iter())
                    .map(|((p, c), m)| (p - 2.0 * c + m) * inv_dt2)
                    .collect()
            });
            return Ok((du, d2u));
        }

        if mode == 1 {
            let s1 = clamp(t + dt);
            let u1 = self.eval_u_d(s1)?;
            let span = s1 - t;
            let du = need_du.then(|| {
                if span.abs() <= 16.0 * f64::EPSILON * scale {
                    zeros()
                } else {
                    u1.iter()
                        .zip(u_d.iter())
                        .map(|(p, c)| (p - c) / span)
                        .collect()
                }
            });
            let d2u = if need_d2u {
                let s2 = clamp(t + 2.0 * dt);
                let u2 = self.eval_u_d(s2)?;
                let h1 = s1 - t;
                let h2 = s2 - s1;
                Some(
                    if h1 <= 16.0 * f64::EPSILON * scale || h2 <= 16.0 * f64::EPSILON * scale {
                        zeros()
                    } else {
                        let inv_dt2 = 1.0 / (dt * dt);
                        u_d.iter()
                            .zip(u1.iter())
                            .zip(u2.iter())
                            .map(|((c, p1), p2)| (c - 2.0 * p1 + p2) * inv_dt2)
                            .collect()
                    },
                )
            } else {
                None
            };
            return Ok((du, d2u));
        }

        let s1 = clamp(t - dt);
        let u1 = self.eval_u_d(s1)?;
        let span = t - s1;
        let du = need_du.then(|| {
            if span.abs() <= 16.0 * f64::EPSILON * scale {
                zeros()
            } else {
                u_d.iter()
                    .zip(u1.iter())
                    .map(|(c, m)| (c - m) / span)
                    .collect()
            }
        });
        let d2u = if need_d2u {
            let s2 = clamp(t - 2.0 * dt);
            let u2 = self.eval_u_d(s2)?;
            let h1 = t - s1;
            let h2 = s1 - s2;
            Some(
                if h1 <= 16.0 * f64::EPSILON * scale || h2 <= 16.0 * f64::EPSILON * scale {
                    zeros()
                } else {
                    let inv_dt2 = 1.0 / (dt * dt);
                    u_d.iter()
                        .zip(u1.iter())
                        .zip(u2.iter())
                        .map(|((c, m1), m2)| (c - 2.0 * m1 + m2) * inv_dt2)
                        .collect()
                },
            )
        } else {
            None
        };
        Ok((du, d2u))
    }
}

impl Model for TimePdeModel<'_> {
    fn f(&mut self, t: f64, y: &[f64]) -> Result<Vec<f64>> {
        self.e.check_cancel()?;
        self.check_frozen(t)?;
        let nf = self.n_free;
        let f_eff = self.eval_f_eff(t)?;
        if !self.is_hyperbolic {
            let mut rhs = f_eff;
            sparse_matvec_add(self.k_ii, y, -1.0, &mut rhs);
            Ok(rhs)
        } else {
            let u = &y[..nf];
            let v = &y[nf..];
            let mut rhs = Vec::with_capacity(2 * nf);
            rhs.extend_from_slice(v);
            let mut acc = f_eff;
            sparse_matvec_add(self.k_ii, u, -1.0, &mut acc);
            if let Some(m_d_ii) = self.m_d_ii {
                sparse_matvec_add(m_d_ii, v, -1.0, &mut acc);
            }
            rhs.extend_from_slice(&acc);
            Ok(rhs)
        }
    }

    fn jac(&mut self, _: f64, _: &[f64]) -> Result<Vec<f64>> {
        Err("solvepde uses a constant Jacobian matrix".into())
    }

    fn mass(&mut self, _: f64, _: &[f64]) -> Result<Vec<f64>> {
        Err("solvepde uses a constant mass matrix".into())
    }

    fn events(&mut self, _: f64, _: &[f64]) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>)> {
        Err("solvepde does not use ODE events".into())
    }

    fn check_cancel(&self) -> Result<()> {
        self.e.check_cancel()
    }
}

fn coeff_mixed_across_faces(has_quad: &[bool], nonzero: &[bool]) -> bool {
    let mut saw_zero = false;
    let mut saw_nonzero = false;
    for (&has, &nz) in has_quad.iter().zip(nonzero.iter()) {
        if !has {
            continue;
        }
        if nz {
            saw_nonzero = true;
        } else {
            saw_zero = true;
        }
    }
    saw_zero && saw_nonzero
}

fn push_nodal_column(data: &mut Vec<f64>, nodal: &[f64], dir_dofs: &[usize], u_d: &[f64]) {
    let start = data.len();
    data.extend_from_slice(nodal);
    for &dof in dir_dofs {
        data[start + dof] = u_d[dof];
    }
}

fn build_time_matrix(n_nodes: usize, n_comp: usize, nt: usize, data: Vec<f64>) -> Result<Matrix> {
    if n_comp == 1 {
        Matrix::from_column_vec(n_nodes, nt, data)
    } else {
        Matrix::from_column_vec(n_nodes, n_comp * nt, data)?.with_dims(&[n_nodes, n_comp, nt])
    }
}

// ---------------------------------------------------------------------------
// Time-dependent solvepde(model, tlist)

#[allow(clippy::needless_range_loop)]
pub(crate) fn solve_pde_time(
    e: &mut Engine,
    model_id: u64,
    tlist_val: &Value,
    out: &mut Execution,
) -> Result<Value> {
    // 1. Validate tlist.
    let tmat = numeric_matrix(tlist_val, "solvepde: tlist")?;
    if tmat.len() < 2 || (tmat.rows() != 1 && tmat.cols() != 1) {
        return Err(
            "solvepde: tlist must be a vector of at least two strictly increasing real finite values."
                .into(),
        );
    }
    let tlist: Vec<f64> = tmat.data().to_vec();
    if tlist.windows(2).any(|w| w[0] >= w[1]) {
        return Err("solvepde: tlist must be strictly increasing (tlist(k+1) > tlist(k)).".into());
    }
    let t0 = tlist[0];
    let t_end = *tlist.last().unwrap();
    let duration = t_end - t0;
    if !duration.is_finite() || duration <= 0.0 {
        return Err("solvepde: tlist time interval must be positive and finite.".into());
    }

    // 2. Extract model state.
    let (
        system_size,
        analytic,
        mesh_arr,
        mesh,
        coeff_assignments,
        bc_assignments,
        ic_assignments,
        solver_options,
    ) = {
        let model = objects::pde_model_get(e, model_id)
            .ok_or("solvepde: invalid or deleted pde.PDEModel")?;
        let analytic = model.geometry.as_ref().map(|(g, _)| g.clone()).ok_or(
            "solvepde: The model has no geometry. Attach a geometry using geometryFromEdges.",
        )?;
        let mesh_arr = model
            .mesh
            .as_ref()
            .cloned()
            .ok_or("solvepde: The model has no mesh. Generate a mesh using generateMesh.")?;
        let mesh_val = Value::Container(mesh_arr.as_ref().clone());
        let mesh: Arc<FeMesh> = objects::pde_mesh_value(e, &mesh_val)
            .ok_or("solvepde: model.Mesh is not a valid pde.FEMesh")?;
        if model.coeff_assignments.is_empty() {
            return Err(
                "solvepde: The model has no equation coefficients. Specify coefficients using specifyCoefficients."
                    .into(),
            );
        }
        if model.ic_assignments.is_empty() {
            return Err(
                "solvepde: Initial conditions must be specified for a time-dependent PDE. Use setInitialConditions."
                    .into(),
            );
        }
        (
            model.system_size,
            analytic,
            mesh_arr,
            mesh,
            model.coeff_assignments.clone(),
            model.bc_assignments.clone(),
            model.ic_assignments.clone(),
            model.solver_options,
        )
    };

    let n_comp = system_size;
    let num_faces = analytic.geom.num_faces;
    let num_edges = analytic.geom.edges.len();
    let n_nodes = mesh.nodes.len();
    let n_dofs = n_nodes * n_comp;

    // 3. Resolve winning coefficient assignment per 1-based face.
    let mut face_coeff: Vec<Option<usize>> = vec![None; num_faces];
    for (idx, ca) in coeff_assignments.iter().enumerate() {
        match &ca.faces {
            None => {
                face_coeff.fill(Some(idx));
            }
            Some(ids) => {
                for &id in ids {
                    if id < 1 || id > num_faces {
                        return Err(format!(
                            "solvepde: coefficient Face ID {id} exceeds geometry NumFaces ({num_faces})"
                        ));
                    }
                    face_coeff[id - 1] = Some(idx);
                }
            }
        }
    }
    for (fi, slot) in face_coeff.iter().enumerate() {
        if slot.is_none() {
            return Err(format!(
                "solvepde: no equation coefficients specified for face {}",
                fi + 1
            ));
        }
    }

    // 4. Evaluate domain coefficients at t0 and check time dependence.
    let tri_face = element_faces(&mesh);
    let (dom_pts, _nq) = collect_domain_quad_points(&mesh, &tri_face, &e.cancel)?;
    let n_dom = dom_pts.len();

    let mut dom_groups: Vec<Vec<usize>> = vec![Vec::new(); coeff_assignments.len()];
    for (q_idx, pt) in dom_pts.iter().enumerate() {
        if pt.subdomain < 1 || pt.subdomain > num_faces {
            return Err(format!(
                "solvepde: element has invalid face ID {}",
                pt.subdomain
            ));
        }
        let ca_idx = face_coeff[pt.subdomain - 1].unwrap();
        dom_groups[ca_idx].push(q_idx);
    }

    check_coeff_workspace(n_dom, n_comp)?;
    let c_stride = 4 * n_comp * n_comp;
    let m_stride = n_comp * n_comp;
    let mut m_vals = vec![0.0; n_dom * m_stride];
    let mut d_vals = vec![0.0; n_dom * m_stride];
    let mut c_vals = vec![0.0; n_dom * c_stride];
    let mut a_vals = vec![0.0; n_dom * m_stride];
    let mut f_vals = vec![0.0; n_dom * n_comp];

    let mut dom_xs: Vec<Vec<f64>> = vec![Vec::new(); coeff_assignments.len()];
    let mut dom_ys: Vec<Vec<f64>> = vec![Vec::new(); coeff_assignments.len()];
    let mut dom_subs: Vec<Vec<f64>> = vec![Vec::new(); coeff_assignments.len()];
    let mut f_time_dep = false;
    let mut frozen: Vec<FrozenCoeff> = Vec::new();

    for (ca_idx, indices) in dom_groups.iter().enumerate() {
        if indices.is_empty() {
            continue;
        }
        let ca = &coeff_assignments[ca_idx];
        let xs: Vec<f64> = indices.iter().map(|&i| dom_pts[i].x).collect();
        let ys: Vec<f64> = indices.iter().map(|&i| dom_pts[i].y).collect();
        let subs: Vec<f64> = indices
            .iter()
            .map(|&i| dom_pts[i].subdomain as f64)
            .collect();

        let mv = eval_mat_sys_with_state(
            e,
            out,
            &ca.m,
            n_comp,
            &xs,
            &ys,
            &subs,
            None,
            t0,
            None,
            "coefficient 'm'",
        )?;
        let dv = eval_mat_sys_with_state(
            e,
            out,
            &ca.d,
            n_comp,
            &xs,
            &ys,
            &subs,
            None,
            t0,
            None,
            "coefficient 'd'",
        )?;
        let cv = eval_c_sys_with_state(e, out, &ca.c, n_comp, &xs, &ys, &subs, t0, None)?;
        let av = eval_mat_sys_with_state(
            e,
            out,
            &ca.a,
            n_comp,
            &xs,
            &ys,
            &subs,
            None,
            t0,
            None,
            "coefficient 'a'",
        )?;
        let fv = eval_vec_sys_with_state(
            e,
            out,
            &ca.f,
            n_comp,
            &xs,
            &ys,
            &subs,
            None,
            t0,
            None,
            "coefficient 'f'",
        )?;

        // Handles that take `state` are re-evaluated with state.time at every step:
        // f enters the load vector; m, d, a, c must stay equal to their t0 values.
        if coeff_takes_state(&ca.f) {
            f_time_dep = true;
        }
        for (kind, takes, vals) in [
            (FrozenKind::M, coeff_takes_state(&ca.m), &mv),
            (FrozenKind::D, coeff_takes_state(&ca.d), &dv),
            (FrozenKind::A, coeff_takes_state(&ca.a), &av),
            (
                FrozenKind::C,
                matches!(&ca.c, CoeffC::Handle(h) if handle_takes_state(h)),
                &cv,
            ),
        ] {
            if takes {
                frozen.push(FrozenCoeff {
                    ca_idx,
                    kind,
                    t0_vals: vals.clone(),
                });
            }
        }

        for (local_k, &q_idx) in indices.iter().enumerate() {
            m_vals[q_idx * m_stride..(q_idx + 1) * m_stride]
                .copy_from_slice(&mv[local_k * m_stride..(local_k + 1) * m_stride]);
            d_vals[q_idx * m_stride..(q_idx + 1) * m_stride]
                .copy_from_slice(&dv[local_k * m_stride..(local_k + 1) * m_stride]);
            c_vals[q_idx * c_stride..(q_idx + 1) * c_stride]
                .copy_from_slice(&cv[local_k * c_stride..(local_k + 1) * c_stride]);
            a_vals[q_idx * m_stride..(q_idx + 1) * m_stride]
                .copy_from_slice(&av[local_k * m_stride..(local_k + 1) * m_stride]);
            f_vals[q_idx * n_comp..(q_idx + 1) * n_comp]
                .copy_from_slice(&fv[local_k * n_comp..(local_k + 1) * n_comp]);
        }
        dom_xs[ca_idx] = xs;
        dom_ys[ca_idx] = ys;
        dom_subs[ca_idx] = subs;
    }

    let mut face_has_quad = vec![false; num_faces];
    let mut face_m_nonzero = vec![false; num_faces];
    let mut face_d_nonzero = vec![false; num_faces];
    for (q_idx, pt) in dom_pts.iter().enumerate() {
        let fi = pt.subdomain - 1;
        if fi >= num_faces {
            continue;
        }
        face_has_quad[fi] = true;
        if m_vals[q_idx * m_stride..(q_idx + 1) * m_stride]
            .iter()
            .any(|&v| v != 0.0)
        {
            face_m_nonzero[fi] = true;
        }
        if d_vals[q_idx * m_stride..(q_idx + 1) * m_stride]
            .iter()
            .any(|&v| v != 0.0)
        {
            face_d_nonzero[fi] = true;
        }
    }

    let any_m = m_vals.iter().any(|&v| v != 0.0);
    let any_d = d_vals.iter().any(|&v| v != 0.0);
    if !any_m && !any_d {
        return Err(
            "solvepde: Cannot specify tlist for a stationary PDE (both 'm' and 'd' are zero). Call solvepde(model) without tlist."
                .into(),
        );
    }
    if coeff_mixed_across_faces(&face_has_quad, &face_m_nonzero)
        || coeff_mixed_across_faces(&face_has_quad, &face_d_nonzero)
    {
        return Err(
            "solvepde: 'm' and 'd' must be nonzero on every face when nonzero on any face".into(),
        );
    }
    let is_hyperbolic = any_m;

    // 5. Assemble domain K, M_a, F, M_d, M_m.
    let sys = assemble_kmf_sys(&mesh, n_comp, &c_vals, &a_vals, &f_vals, &e.cancel)?;
    let mut total_k = sys.k.add(&sys.m, 1.0)?;
    let dom_f = sys.f;
    let m_d_full = if any_d {
        Some(assemble_mass_sys(&mesh, n_comp, &d_vals, &e.cancel)?)
    } else {
        None
    };
    let m_m_full = if any_m {
        Some(assemble_mass_sys(&mesh, n_comp, &m_vals, &e.cancel)?)
    } else {
        None
    };

    // 6. Resolve winning BC assignment per (edge, comp).
    let edge_comp_bc = resolve_edge_comp_bcs(&bc_assignments, num_edges, n_comp, "solvepde")?;

    let all_facets = extract_boundary_facets(&mesh, &analytic.geom);
    let mut neumann_facets: Vec<BoundaryFacet> = Vec::new();
    let mut facet_edges: Vec<usize> = Vec::new();
    for f in all_facets {
        let has_neu = edge_comp_bc[f.edge_id - 1].iter().any(|opt| {
            opt.is_some_and(|bc_idx| {
                matches!(
                    bc_assignments[bc_idx].kind,
                    BcKind::Neumann { .. } | BcKind::Mixed { .. }
                )
            })
        });
        if has_neu {
            facet_edges.push(f.edge_id);
            neumann_facets.push(f);
        }
    }

    let (edge_pts, nq_edge) = if !neumann_facets.is_empty() {
        collect_edge_quad_points(&mesh, &neumann_facets)?
    } else {
        (Vec::new(), 0)
    };
    let mut g_time_dep = false;
    let mut neu_g = vec![0.0; n_dofs];
    let mut q_check = false;
    let mut q_t0: Option<Vec<f64>> = None;

    if !neumann_facets.is_empty() {
        // Active Neumann g handles that take `state` are re-evaluated every step;
        // q handles that take `state` are checked against their t0 values.
        for (bc_idx, bc) in bc_assignments.iter().enumerate() {
            let (g_opt, q_opt) = match &bc.kind {
                BcKind::Neumann { g, q, .. } => (Some(g), Some(q)),
                BcKind::Mixed { g, q, .. } => (g.as_ref(), q.as_ref()),
                _ => continue,
            };
            let active = facet_edges
                .iter()
                .any(|&eid| edge_comp_bc[eid - 1].contains(&Some(bc_idx)));
            if !active {
                continue;
            }
            q_check |= q_opt.is_some_and(coeff_takes_state);
            g_time_dep |= g_opt.is_some_and(coeff_takes_state);
        }

        let (q_vals, g_vals) = eval_neumann_on_edge_pts(
            e,
            out,
            n_comp,
            &bc_assignments,
            &edge_comp_bc,
            &facet_edges,
            nq_edge,
            &edge_pts,
            t0,
            None,
            false,
        )?;
        let (q_mat, g_vec) =
            assemble_neumann_sys(&mesh, n_comp, &neumann_facets, &q_vals, &g_vals, &e.cancel)?;
        if q_mat.nnz() > 0 {
            total_k = total_k.add(&q_mat, 1.0)?;
        }
        neu_g = g_vec;
        if q_check {
            q_t0 = Some(q_vals);
        }
    }

    // Active Dirichlet u/h/r handles that take `state` are re-evaluated every step.
    let mut dir_time_dep = false;
    for (bc_idx, bc) in bc_assignments.iter().enumerate() {
        let active = (1..=num_edges).any(|eid| edge_comp_bc[eid - 1].contains(&Some(bc_idx)));
        if !active {
            continue;
        }
        dir_time_dep |= match &bc.kind {
            BcKind::DirichletU { u, .. } => coeff_takes_state(u),
            BcKind::DirichletHR { h, r, .. } => coeff_takes_state(h) || coeff_takes_state(r),
            BcKind::Mixed { u, h, r, .. } => [u, h, r]
                .into_iter()
                .any(|c| c.as_ref().is_some_and(coeff_takes_state)),
            BcKind::Neumann { .. } => false,
        };
    }

    let (dirichlet_mask, u_d_t0_full) = evaluate_dirichlet_sys(
        e,
        out,
        &mesh,
        &analytic.geom,
        n_comp,
        &bc_assignments,
        &edge_comp_bc,
        t0,
        false,
    )?;

    // 7. Evaluate initial conditions.
    let (u0_full, ut0_full) =
        evaluate_initial_conditions(e, out, &mesh, n_comp, num_faces, &ic_assignments)?;

    // Partition DOFs into free and Dirichlet.
    let mut free_dofs = Vec::with_capacity(n_dofs);
    let mut dir_dofs = Vec::new();
    let mut node_to_free = vec![usize::MAX; n_dofs];
    for i in 0..n_dofs {
        if dirichlet_mask[i] {
            dir_dofs.push(i);
        } else {
            node_to_free[i] = free_dofs.len();
            free_dofs.push(i);
        }
    }
    let n_free = free_dofs.len();
    let n_dir = dir_dofs.len();
    let nt = tlist.len();

    if n_dofs.saturating_mul(nt) > ivp::MAX_VALUES {
        return Err(format!(
            "solvepde: output size {}x{} exceeds the {}-element limit",
            n_dofs,
            nt,
            ivp::MAX_VALUES
        ));
    }

    // Edge case: all DOFs are Dirichlet.
    if n_free == 0 {
        let mut data = Vec::with_capacity(n_dofs * nt);
        push_nodal_column(&mut data, &u0_full, &dir_dofs, &u_d_t0_full);
        for &tk in &tlist[1..] {
            let ud = if dir_time_dep {
                evaluate_dirichlet_sys(
                    e,
                    out,
                    &mesh,
                    &analytic.geom,
                    n_comp,
                    &bc_assignments,
                    &edge_comp_bc,
                    tk,
                    false,
                )?
                .1
            } else {
                u_d_t0_full.clone()
            };
            data.extend_from_slice(&ud);
        }
        let nodal_solution = build_time_matrix(n_nodes, n_comp, nt, data)?;
        let (x_gradients, y_gradients) =
            compute_nodal_gradients(&mesh, &nodal_solution, &e.cancel)?;
        let solution_times = Matrix::from_column_vec(1, nt, tlist)?;
        let results = TimeDependentResults {
            nodal_solution,
            solution_times,
            x_gradients,
            y_gradients,
            mesh,
            mesh_arr,
        };
        return objects::pde_time_dependent_results_new(e, Arc::new(results));
    }

    let n_sys = if is_hyperbolic { 2 * n_free } else { n_free };
    if n_sys > MAX_TIME_DOFS {
        return Err(format!(
            "solvepde: time-dependent system has {n_sys} state DOFs, exceeding the {MAX_TIME_DOFS}-DOF limit; use a coarser mesh"
        ));
    }

    // 8. Partition matrices into (II, ID).
    let (k_ii, k_id) = partition_matrix(
        &total_k,
        &dirichlet_mask,
        &node_to_free,
        n_free,
        n_dir,
        &e.cancel,
    )?;
    let m_d_parts = match &m_d_full {
        Some(md) => Some(partition_matrix(
            md,
            &dirichlet_mask,
            &node_to_free,
            n_free,
            n_dir,
            &e.cancel,
        )?),
        None => None,
    };
    let m_m_parts = match &m_m_full {
        Some(mm) => Some(partition_matrix(
            mm,
            &dirichlet_mask,
            &node_to_free,
            n_free,
            n_dir,
            &e.cancel,
        )?),
        None => None,
    };

    let mut mass_dense = vec![0.0; n_sys * n_sys];
    let mut jac_dense = vec![0.0; n_sys * n_sys];
    if !is_hyperbolic {
        let (m_d_ii, _) = m_d_parts.as_ref().unwrap();
        sparse_scatter_dense(m_d_ii, 1.0, &mut mass_dense, n_sys, 0, 0);
        sparse_scatter_dense(&k_ii, -1.0, &mut jac_dense, n_sys, 0, 0);
    } else {
        let (m_m_ii, _) = m_m_parts.as_ref().unwrap();
        for j in 0..n_free {
            mass_dense[j * n_sys + j] = 1.0;
            jac_dense[(n_free + j) * n_sys + j] = 1.0;
        }
        sparse_scatter_dense(m_m_ii, 1.0, &mut mass_dense, n_sys, n_free, n_free);
        sparse_scatter_dense(&k_ii, -1.0, &mut jac_dense, n_sys, n_free, 0);
        if let Some((m_d_ii, _)) = &m_d_parts {
            sparse_scatter_dense(m_d_ii, -1.0, &mut jac_dense, n_sys, n_free, n_free);
        }
    }

    let mut y0 = Vec::with_capacity(n_sys);
    for &dof in &free_dofs {
        y0.push(u0_full[dof]);
    }
    if is_hyperbolic {
        for &dof in &free_dofs {
            y0.push(ut0_full[dof]);
        }
    }

    let const_u_d: Vec<f64> = dir_dofs.iter().map(|&d| u_d_t0_full[d]).collect();
    let base_f_i: Vec<f64> = free_dofs
        .iter()
        .map(|&i| {
            let mut s = 0.0;
            if !f_time_dep {
                s += dom_f[i];
            }
            if !g_time_dep {
                s += neu_g[i];
            }
            s
        })
        .collect();
    let const_f_eff = if !f_time_dep && !g_time_dep && !dir_time_dep {
        let mut fe = base_f_i.clone();
        if !const_u_d.is_empty() {
            sparse_matvec_add(&k_id, &const_u_d, -1.0, &mut fe);
        }
        Some(fe)
    } else {
        None
    };

    // 9. Call ode15s.
    let mut model_adapter = TimePdeModel {
        e,
        out,
        mesh: &mesh,
        geom: &analytic.geom,
        n_comp,
        is_hyperbolic,
        n_free,
        free_dofs: &free_dofs,
        dir_dofs: &dir_dofs,
        k_ii: &k_ii,
        k_id: &k_id,
        m_d_ii: m_d_parts.as_ref().map(|(ii, _)| ii),
        m_d_id: m_d_parts.as_ref().map(|(_, id)| id),
        m_m_id: m_m_parts.as_ref().map(|(_, id)| id),
        const_f_eff,
        base_f_i,
        f_time_dep,
        dom_xs,
        dom_ys,
        dom_subs,
        dom_groups,
        n_dom,
        coeff_assignments: &coeff_assignments,
        g_time_dep,
        neumann_facets: &neumann_facets,
        edge_pts: &edge_pts,
        facet_edges: &facet_edges,
        nq_edge,
        edge_comp_bc: &edge_comp_bc,
        dir_time_dep,
        bc_assignments: &bc_assignments,
        const_u_d,
        t_lo: t0,
        t_hi: t_end,
        frozen,
        q_t0,
        checked_t: f64::NAN,
    };

    let f0 = model_adapter.f(t0, &y0)?;
    let SolverOptions {
        relative_tolerance,
        absolute_tolerance,
        ..
    } = solver_options;
    let rtol = relative_tolerance.max(100.0 * f64::EPSILON);
    let atol = vec![absolute_tolerance; n_sys];
    let threshold = atol.iter().map(|a| a / rtol).collect();

    let opts = Opts {
        rtol,
        atol,
        normcontrol: false,
        threshold,
        hmax: duration / 10.0,
        htry: None,
        htspan: tlist[1] - tlist[0],
        maxorder: 5,
        bdf: false,
        jac: JacKind::Matrix(jac_dense),
        jconstant: true,
        mass: MassKind::Matrix(mass_dense),
        mass_singular: "no".into(),
        initial_slope: None,
        events: false,
        non_negative: Vec::new(),
    };

    let ode_out = ivp::ode15s(
        &mut model_adapter,
        Setup {
            tspan: &tlist,
            y0: &y0,
            f0,
            valt: None,
            m0: None,
            at: OutAt::Requested,
        },
        &opts,
    )?;

    if ode_out.failure.is_some() || ode_out.t.len() != nt {
        return Err(format!(
            "solvepde: time integration failed before the final requested time: {}",
            ode_out.warnings.join(" ")
        ));
    }

    // 10. Reconstruct NodalSolution (N x Nt for N_comp == 1, N x N_comp x Nt for N_comp > 1).
    let mut data = Vec::with_capacity(n_dofs * nt);
    push_nodal_column(&mut data, &u0_full, &dir_dofs, &u_d_t0_full);

    for k in 1..nt {
        e.check_cancel()?;
        let yk = &ode_out.y[k];
        let mut col = vec![0.0; n_dofs];
        for (fi, &dof) in free_dofs.iter().enumerate() {
            let v = yk[fi];
            if !v.is_finite() {
                return Err("solvepde: time integration produced non-finite values".into());
            }
            col[dof] = v;
        }
        if !dir_dofs.is_empty() {
            if dir_time_dep {
                let (_, ud) = evaluate_dirichlet_sys(
                    e,
                    out,
                    &mesh,
                    &analytic.geom,
                    n_comp,
                    &bc_assignments,
                    &edge_comp_bc,
                    tlist[k],
                    false,
                )?;
                for &dof in &dir_dofs {
                    col[dof] = ud[dof];
                }
            } else {
                for &dof in &dir_dofs {
                    col[dof] = u_d_t0_full[dof];
                }
            }
        }
        data.extend_from_slice(&col);
    }

    let nodal_solution = build_time_matrix(n_nodes, n_comp, nt, data)?;
    let (x_gradients, y_gradients) = compute_nodal_gradients(&mesh, &nodal_solution, &e.cancel)?;
    let solution_times = Matrix::from_column_vec(1, nt, tlist)?;
    let results = TimeDependentResults {
        nodal_solution,
        solution_times,
        x_gradients,
        y_gradients,
        mesh,
        mesh_arr,
    };
    objects::pde_time_dependent_results_new(e, Arc::new(results))
}
