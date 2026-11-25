//! 2-D PDE coefficient specification (`specifyCoefficients`), boundary
//! conditions (`applyBoundaryCondition`), stationary linear & nonlinear solver (`solvepde`),
//! stationary results object (`pde.StationaryResults`), and post-processing
//! (`interpolateSolution`, `evaluateGradient`).
pub(crate) mod assemble;
pub(crate) mod eigen;
pub(crate) mod nonlinear;
pub(crate) mod postproc;
pub(crate) mod quadrature;
pub(crate) mod time;
pub(crate) mod types;

pub(crate) use eigen::{EIGEN_RESULTS_CLASS, EIGEN_RESULTS_PROPERTIES, EigenResults};

use super::{Engine, Execution, Value, objects, pde_mesh::FeMesh};
use crate::{Array, FunctionHandle, Matrix, Result, StructArray};
use assemble::{
    BoundaryFacet, EdgeQuadPoint, apply_dirichlet_elimination, assemble_kmf_sys,
    assemble_neumann_sys, collect_domain_quad_points, collect_edge_quad_points, element_faces,
    extract_boundary_facets, reconstruct_solution, solve_sparse_system,
};
use nalgebra::{DMatrix, DVector};
use nonlinear::QuadState;
use std::sync::Arc;
use types::{
    BcAssignment, BcKind, CoeffAssignment, CoeffC, CoeffVal, bc_records_array, coeff_records_array,
    expand_mat_coeff, expand_vec_coeff, numeric_matrix, text_value, valid_c_coeff_len,
    valid_mat_coeff_len,
};

pub(crate) const RESULTS_CLASS: &str = "pde.StationaryResults";
pub(crate) const RESULTS_PROPERTIES: [&str; 4] =
    ["NodalSolution", "XGradients", "YGradients", "Mesh"];

/// Maximum points evaluated per function-handle batch call.
pub(crate) const EVAL_CHUNK: usize = 8192;

/// Largest dense coefficient buffer a system solve may allocate.
/// `n_dom * 4 * N^2` doubles for `c`, plus the matching `a`/`d`/`f` eval buffers.
pub(crate) const MAX_COEFF_WORKSPACE: usize = 1 << 27;

/// Reject a system whose dense `c` workspace (`n_dom * 4 * N^2` doubles) exceeds
/// [`MAX_COEFF_WORKSPACE`], before any of those buffers are allocated.
pub(crate) fn check_coeff_workspace(n_dom: usize, n_comp: usize) -> Result<()> {
    let elems = n_dom
        .checked_mul(4)
        .and_then(|n| n.checked_mul(n_comp))
        .and_then(|n| n.checked_mul(n_comp));
    match elems {
        Some(n) if n <= MAX_COEFF_WORKSPACE => Ok(()),
        _ => Err(format!(
            "system too large: coefficient workspace exceeds {MAX_COEFF_WORKSPACE} elements"
        )),
    }
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if name == "evaluateGradient" {
        return postproc::evaluate_gradient(e, args, count);
    }
    if count > 1 {
        return Err(format!("{name}: too many output arguments"));
    }
    let val = match name {
        "specifyCoefficients" => specify_coefficients(e, args)?,
        "applyBoundaryCondition" => apply_boundary_condition(e, args)?,
        "setInitialConditions" => time::set_initial_conditions(e, args)?,
        "solvepde" => solve_pde(e, args, out)?,
        "solvepdeeig" => eigen::solve_pde_eig(e, args, out)?,
        "interpolateSolution" => postproc::interpolate_solution(e, args)?,
        _ => return Err(format!("Unknown PDE solver function {name}")),
    };
    Ok(if count == 0 { vec![] } else { vec![val] })
}

// ---------------------------------------------------------------------------
// pde.StationaryResults (value semantics)

#[derive(Clone, Debug)]
pub(crate) struct StationaryResults {
    pub(crate) nodal_solution: Matrix,
    pub(crate) x_gradients: Matrix,
    pub(crate) y_gradients: Matrix,
    pub(crate) mesh: Arc<FeMesh>,
    pub(crate) mesh_arr: Arc<Array>,
}

impl PartialEq for StationaryResults {
    fn eq(&self, other: &Self) -> bool {
        self.nodal_solution.rows() == other.nodal_solution.rows()
            && self.nodal_solution.cols() == other.nodal_solution.cols()
            && self.nodal_solution.data() == other.nodal_solution.data()
            && self.mesh == other.mesh
    }
}

impl StationaryResults {
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + (self.nodal_solution.len() + self.x_gradients.len() + self.y_gradients.len())
                * std::mem::size_of::<f64>()
            + self.mesh_arr.retained_bytes()
    }

    pub(crate) fn push_children<'a>(&'a self, pending: &mut Vec<&'a Array>) {
        pending.push(self.mesh_arr.as_ref());
    }

    pub(crate) fn property(&self, name: &str) -> Result<Value> {
        match name {
            "NodalSolution" => Ok(Value::Matrix(self.nodal_solution.clone())),
            "XGradients" => Ok(Value::Matrix(self.x_gradients.clone())),
            "YGradients" => Ok(Value::Matrix(self.y_gradients.clone())),
            "Mesh" => Ok(Value::Container(self.mesh_arr.as_ref().clone())),
            _ => Err(format!(
                "No appropriate method, property, or field '{name}' for class '{RESULTS_CLASS}'."
            )),
        }
    }

    pub(crate) fn property_ci(&self, name: &str) -> Result<Value> {
        if let Some(&canonical) = RESULTS_PROPERTIES
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
                let mut fields = Vec::with_capacity(RESULTS_PROPERTIES.len());
                for &prop in &RESULTS_PROPERTIES {
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
                    text_value(prop).ok_or("pde.StationaryResults: property name must be text")?;
                self.property_ci(&name)
            }
            _ => Err("pde.StationaryResults: get expects zero or one property name".into()),
        }
    }

    pub(crate) fn body(&self) -> String {
        format!(
            "  StationaryResults with properties:\n\n\
             {:>18}: [{}x{} double]\n\
             {:>18}: [{}x{} double]\n\
             {:>18}: [{}x{} double]\n\
             {:>18}: [1x1 FEMesh]\n",
            "NodalSolution",
            self.nodal_solution.rows(),
            self.nodal_solution.cols(),
            "XGradients",
            self.x_gradients.rows(),
            self.x_gradients.cols(),
            "YGradients",
            self.y_gradients.rows(),
            self.y_gradients.cols(),
            "Mesh",
        )
    }
}

/// Format matrix dimensions as e.g. `100x2` or `100x2x5` for object display.
pub(crate) fn format_dims(m: &Matrix) -> String {
    let dims = m.dims();
    if dims.len() > 2 {
        dims.iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("x")
    } else {
        format!("{}x{}", m.rows(), m.cols())
    }
}

// ---------------------------------------------------------------------------
// specifyCoefficients

fn parse_mat_coeff(v: &Value, n: usize, name: &str) -> Result<CoeffVal> {
    if let Value::Container(Array::Handle(h)) = v {
        return Ok(CoeffVal::Handle(h.clone()));
    }
    let m = numeric_matrix(v, &format!("specifyCoefficients: '{name}'"))?;
    if m.rows() == n && m.cols() == n {
        return Ok(CoeffVal::Const(m.data().to_vec()));
    }
    if m.cols() != 1 || !valid_mat_coeff_len(m.rows(), n) {
        return Err(format!(
            "specifyCoefficients: '{name}' must be a scalar, {n}x1, {}x1, or {}x1 column vector, {n}x{n} matrix, or function handle",
            n * (n + 1) / 2,
            n * n
        ));
    }
    Ok(CoeffVal::Const(m.data().to_vec()))
}

fn parse_vec_coeff(v: &Value, n: usize, name: &str) -> Result<CoeffVal> {
    if let Value::Container(Array::Handle(h)) = v {
        return Ok(CoeffVal::Handle(h.clone()));
    }
    let m = numeric_matrix(v, &format!("specifyCoefficients: '{name}'"))?;
    if m.len() == 1 || (m.cols() == 1 && m.rows() == n) {
        return Ok(CoeffVal::Const(m.data().to_vec()));
    }
    Err(format!(
        "specifyCoefficients: '{name}' must be a scalar, {n}x1 column vector, or function handle"
    ))
}

fn parse_c_coeff(v: &Value, n: usize) -> Result<CoeffC> {
    if let Value::Container(Array::Handle(h)) = v {
        return Ok(CoeffC::Handle(h.clone()));
    }
    let m = numeric_matrix(v, "specifyCoefficients: 'c'")?;
    if m.cols() != 1 || !valid_c_coeff_len(m.rows(), n) {
        if n == 1 {
            return Err(
                "specifyCoefficients: 'c' must be a scalar, 2x1, 3x1, or 4x1 numeric column vector or function handle"
                    .into(),
            );
        }
        return Err(format!(
            "specifyCoefficients: 'c' has invalid row count {} for a {n}-equation system (expected 1, 2, 3, 4, {n}, {}, {}, {}, {}, or {} rows)",
            m.rows(),
            2 * n,
            3 * n,
            4 * n,
            n * (2 * n + 1),
            4 * n * n
        ));
    }
    Ok(CoeffC::Const(m.data().to_vec()))
}

pub(crate) fn parse_region_ids(
    v: &Value,
    max_id: Option<usize>,
    label: &str,
) -> Result<Vec<usize>> {
    let m = numeric_matrix(v, label)?;
    if m.is_empty() || (m.rows() != 1 && m.cols() != 1) {
        return Err(format!(
            "{label} must be a nonempty vector of positive integer IDs"
        ));
    }
    let mut ids = Vec::with_capacity(m.len());
    for &x in m.data() {
        if x < 1.0 || x.fract() != 0.0 || x > usize::MAX as f64 {
            return Err(format!("{label} must contain positive integer IDs"));
        }
        let id = x as usize;
        if let Some(max) = max_id
            && id > max
        {
            return Err(format!(
                "{label}: ID {id} exceeds the maximum region ID ({max})"
            ));
        }
        ids.push(id);
    }
    Ok(ids)
}

/// Recompute `IsTimeDependent` from the effective (last-assigned) coefficients.
pub(crate) fn recompute_is_time_dependent(
    coeff_assignments: &[CoeffAssignment],
    num_faces: Option<usize>,
) -> bool {
    if let Some(nf) = num_faces {
        let mut face_coeff: Vec<Option<usize>> = vec![None; nf];
        for (idx, ca) in coeff_assignments.iter().enumerate() {
            match &ca.faces {
                None => {
                    face_coeff.fill(Some(idx));
                }
                Some(ids) => {
                    for &id in ids {
                        if id >= 1 && id <= nf {
                            face_coeff[id - 1] = Some(idx);
                        }
                    }
                }
            }
        }
        face_coeff
            .into_iter()
            .flatten()
            .any(|idx| coeff_assignments[idx].is_time_dependent())
    } else {
        let mut global_idx: Option<usize> = None;
        let mut face_map: std::collections::HashMap<usize, usize> =
            std::collections::HashMap::new();
        for (idx, ca) in coeff_assignments.iter().enumerate() {
            match &ca.faces {
                None => {
                    global_idx = Some(idx);
                    face_map.clear();
                }
                Some(ids) => {
                    for &id in ids {
                        face_map.insert(id, idx);
                    }
                }
            }
        }
        let global_td = global_idx.is_some_and(|i| coeff_assignments[i].is_time_dependent());
        global_td
            || face_map
                .values()
                .any(|&i| coeff_assignments[i].is_time_dependent())
    }
}

fn specify_coefficients(e: &mut Engine, args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Err(
            "specifyCoefficients: expected specifyCoefficients(model, 'm', m, 'd', d, 'c', c, 'a', a, 'f', f, ...)"
                .into(),
        );
    }
    let model_id = objects::pde_model_id(e, &args[0], "specifyCoefficients")?;
    let (system_size, num_faces) = {
        let model = objects::pde_model_get(e, model_id)
            .ok_or("specifyCoefficients: invalid or deleted pde.PDEModel")?;
        (
            model.system_size,
            model.geometry.as_ref().map(|(g, _)| g.geom.num_faces),
        )
    };

    let nv = &args[1..];
    if !nv.len().is_multiple_of(2) {
        return Err("specifyCoefficients: arguments after model must be name-value pairs".into());
    }

    let mut m_val: Option<CoeffVal> = None;
    let mut d_val: Option<CoeffVal> = None;
    let mut c_val: Option<CoeffC> = None;
    let mut a_val: Option<CoeffVal> = None;
    let mut f_val: Option<CoeffVal> = None;
    let mut faces: Option<Vec<usize>> = None;

    for pair in nv.as_chunks::<2>().0 {
        let key = text_value(&pair[0])
            .ok_or("specifyCoefficients: parameter names must be character vectors or strings")?;
        if key.eq_ignore_ascii_case("m") {
            if m_val.is_some() {
                return Err("specifyCoefficients: duplicate parameter 'm'".into());
            }
            m_val = Some(parse_mat_coeff(&pair[1], system_size, "m")?);
        } else if key.eq_ignore_ascii_case("d") {
            if d_val.is_some() {
                return Err("specifyCoefficients: duplicate parameter 'd'".into());
            }
            d_val = Some(parse_mat_coeff(&pair[1], system_size, "d")?);
        } else if key.eq_ignore_ascii_case("c") {
            if c_val.is_some() {
                return Err("specifyCoefficients: duplicate parameter 'c'".into());
            }
            c_val = Some(parse_c_coeff(&pair[1], system_size)?);
        } else if key.eq_ignore_ascii_case("a") {
            if a_val.is_some() {
                return Err("specifyCoefficients: duplicate parameter 'a'".into());
            }
            a_val = Some(parse_mat_coeff(&pair[1], system_size, "a")?);
        } else if key.eq_ignore_ascii_case("f") {
            if f_val.is_some() {
                return Err("specifyCoefficients: duplicate parameter 'f'".into());
            }
            f_val = Some(parse_vec_coeff(&pair[1], system_size, "f")?);
        } else if key.eq_ignore_ascii_case("Face") {
            if faces.is_some() {
                return Err("specifyCoefficients: duplicate parameter 'Face'".into());
            }
            faces = Some(parse_region_ids(
                &pair[1],
                num_faces,
                "specifyCoefficients: 'Face'",
            )?);
        } else {
            return Err(format!(
                "specifyCoefficients: unknown parameter '{key}' (expected 'm', 'd', 'c', 'a', 'f', or 'Face')"
            ));
        }
    }

    let missing: Vec<&str> = [
        ("m", m_val.is_none()),
        ("d", d_val.is_none()),
        ("c", c_val.is_none()),
        ("a", a_val.is_none()),
        ("f", f_val.is_none()),
    ]
    .into_iter()
    .filter_map(|(k, miss)| miss.then_some(k))
    .collect();
    if !missing.is_empty() {
        return Err(format!(
            "specifyCoefficients: required coefficient(s) missing: {}. All five coefficients 'm', 'd', 'c', 'a', and 'f' must be specified.",
            missing.join(", ")
        ));
    }

    let assignment = CoeffAssignment {
        faces,
        m: m_val.unwrap(),
        d: d_val.unwrap(),
        c: c_val.unwrap(),
        a: a_val.unwrap(),
        f: f_val.unwrap(),
    };
    let ret_struct = assignment.to_struct(&e.cancel)?;
    let cancel = e.cancel.clone();
    objects::pde_model_update(e, model_id, |model| {
        model.coeff_assignments.push(assignment);
        let nf = model.geometry.as_ref().map(|(g, _)| g.geom.num_faces);
        model.is_time_dependent = recompute_is_time_dependent(&model.coeff_assignments, nf);
        let rec = coeff_records_array(&model.coeff_assignments, &cancel)?;
        model.equation_coefficients = Some(Arc::new(rec));
        Ok(())
    })?;

    Ok(Value::Container(ret_struct))
}

// ---------------------------------------------------------------------------
// applyBoundaryCondition

fn parse_bc_vec(v: &Value, n: usize, name: &str) -> Result<CoeffVal> {
    if let Value::Container(Array::Handle(h)) = v {
        return Ok(CoeffVal::Handle(h.clone()));
    }
    let m = numeric_matrix(v, &format!("applyBoundaryCondition: '{name}'"))?;
    if m.len() == 1 || (m.rows() == 1 && m.cols() == n) || (m.cols() == 1 && m.rows() == n) {
        return Ok(CoeffVal::Const(m.data().to_vec()));
    }
    Err(format!(
        "applyBoundaryCondition: '{name}' must be a scalar, length-{n} vector, or function handle"
    ))
}

fn parse_bc_mat(v: &Value, n: usize, name: &str) -> Result<CoeffVal> {
    if let Value::Container(Array::Handle(h)) = v {
        return Ok(CoeffVal::Handle(h.clone()));
    }
    let m = numeric_matrix(v, &format!("applyBoundaryCondition: '{name}'"))?;
    if m.rows() == n && m.cols() == n {
        return Ok(CoeffVal::Const(m.data().to_vec()));
    }
    if m.cols() == 1 && valid_mat_coeff_len(m.rows(), n) {
        return Ok(CoeffVal::Const(m.data().to_vec()));
    }
    if m.rows() == 1 && valid_mat_coeff_len(m.cols(), n) {
        return Ok(CoeffVal::Const(m.data().to_vec()));
    }
    Err(format!(
        "applyBoundaryCondition: '{name}' must be a scalar, {n}x{n} matrix, valid column vector, or function handle"
    ))
}

fn apply_boundary_condition(e: &mut Engine, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        return Err(
            "applyBoundaryCondition: expected applyBoundaryCondition(model, 'dirichlet'|'neumann'|'mixed', 'Edge', ids, ...)"
                .into(),
        );
    }
    let model_id = objects::pde_model_id(e, &args[0], "applyBoundaryCondition")?;
    let (system_size, num_edges) = {
        let model = objects::pde_model_get(e, model_id)
            .ok_or("applyBoundaryCondition: invalid or deleted pde.PDEModel")?;
        (
            model.system_size,
            model.geometry.as_ref().map(|(g, _)| g.geom.edges.len()),
        )
    };

    let bc_type = text_value(&args[1]).ok_or(
        "applyBoundaryCondition: second argument must be 'dirichlet', 'neumann', or 'mixed'",
    )?;
    let mode = if bc_type.eq_ignore_ascii_case("dirichlet") {
        0u8
    } else if bc_type.eq_ignore_ascii_case("neumann") {
        1u8
    } else if bc_type.eq_ignore_ascii_case("mixed") {
        2u8
    } else {
        return Err(format!(
            "applyBoundaryCondition: unknown boundary condition type '{bc_type}' (expected 'dirichlet', 'neumann', or 'mixed')"
        ));
    };

    let nv = &args[2..];
    if !nv.len().is_multiple_of(2) {
        return Err(
            "applyBoundaryCondition: arguments after condition type must be name-value pairs"
                .into(),
        );
    }

    // First pass: find Edge and EquationIndex/Equation so we know target component count.
    let mut edges: Option<Vec<usize>> = None;
    let mut eq_indices: Option<Vec<usize>> = None;
    for pair in nv.as_chunks::<2>().0 {
        let key = text_value(&pair[0]).ok_or(
            "applyBoundaryCondition: parameter names must be character vectors or strings",
        )?;
        if key.eq_ignore_ascii_case("Edge") {
            if edges.is_some() {
                return Err("applyBoundaryCondition: duplicate parameter 'Edge'".into());
            }
            edges = Some(parse_region_ids(
                &pair[1],
                num_edges,
                "applyBoundaryCondition: 'Edge'",
            )?);
        } else if key.eq_ignore_ascii_case("EquationIndex") || key.eq_ignore_ascii_case("Equation")
        {
            if eq_indices.is_some() {
                return Err("applyBoundaryCondition: duplicate parameter 'EquationIndex'".into());
            }
            let ids = parse_region_ids(
                &pair[1],
                Some(system_size),
                "applyBoundaryCondition: 'EquationIndex'",
            )?;
            eq_indices = Some(ids);
        }
    }

    let n_eq = eq_indices.as_ref().map_or(system_size, Vec::len);

    let mut u_val: Option<CoeffVal> = None;
    let mut h_val: Option<CoeffVal> = None;
    let mut r_val: Option<CoeffVal> = None;
    let mut g_val: Option<CoeffVal> = None;
    let mut q_val: Option<CoeffVal> = None;

    for pair in nv.as_chunks::<2>().0 {
        let key = text_value(&pair[0]).unwrap();
        if key.eq_ignore_ascii_case("Edge")
            || key.eq_ignore_ascii_case("EquationIndex")
            || key.eq_ignore_ascii_case("Equation")
        {
            continue;
        } else if key.eq_ignore_ascii_case("u") {
            if mode == 1 {
                return Err(
                    "applyBoundaryCondition: 'u' is only valid for 'dirichlet' or 'mixed' boundary conditions"
                        .into(),
                );
            }
            u_val = Some(parse_bc_vec(&pair[1], n_eq, "u")?);
        } else if key.eq_ignore_ascii_case("h") {
            if mode == 1 {
                return Err(
                    "applyBoundaryCondition: 'h' is only valid for 'dirichlet' or 'mixed' boundary conditions"
                        .into(),
                );
            }
            let target_n = if mode == 2 { system_size } else { n_eq };
            let h = parse_bc_mat(&pair[1], target_n, "h")?;
            if mode == 0 && h.is_zero_const() {
                return Err(
                    "applyBoundaryCondition: Dirichlet coefficient 'h' must be nonzero".into(),
                );
            }
            h_val = Some(h);
        } else if key.eq_ignore_ascii_case("r") {
            if mode == 1 {
                return Err(
                    "applyBoundaryCondition: 'r' is only valid for 'dirichlet' or 'mixed' boundary conditions"
                        .into(),
                );
            }
            let target_n = if mode == 2 { system_size } else { n_eq };
            r_val = Some(parse_bc_vec(&pair[1], target_n, "r")?);
        } else if key.eq_ignore_ascii_case("g") {
            if mode == 0 {
                return Err(
                    "applyBoundaryCondition: 'g' is only valid for 'neumann' or 'mixed' boundary conditions"
                        .into(),
                );
            }
            let target_n = if mode == 2 { system_size } else { n_eq };
            g_val = Some(parse_bc_vec(&pair[1], target_n, "g")?);
        } else if key.eq_ignore_ascii_case("q") {
            if mode == 0 {
                return Err(
                    "applyBoundaryCondition: 'q' is only valid for 'neumann' or 'mixed' boundary conditions"
                        .into(),
                );
            }
            let target_n = if mode == 2 { system_size } else { n_eq };
            q_val = Some(parse_bc_mat(&pair[1], target_n, "q")?);
        } else {
            return Err(format!("applyBoundaryCondition: unknown parameter '{key}'"));
        }
    }

    let edges = edges
        .ok_or("applyBoundaryCondition: region parameter 'Edge' is required for 2-D geometries")?;

    let kind = match mode {
        0 => match (u_val, h_val, r_val) {
            (Some(u), None, None) => BcKind::DirichletU { u, eq_indices },
            (Some(_), _, _) => {
                return Err(
                    "applyBoundaryCondition: cannot combine 'u' with 'h' or 'r' in a Dirichlet condition"
                        .into(),
                );
            }
            (None, None, None) => {
                return Err(
                    "applyBoundaryCondition: Dirichlet boundary condition requires 'u' or 'h'/'r'"
                        .into(),
                );
            }
            (None, h, r) => BcKind::DirichletHR {
                h: h.unwrap_or_else(|| CoeffVal::scalar(1.0)),
                r: r.unwrap_or_else(|| CoeffVal::scalar(0.0)),
                eq_indices,
            },
        },
        1 => BcKind::Neumann {
            g: g_val.unwrap_or_else(|| CoeffVal::scalar(0.0)),
            q: q_val.unwrap_or_else(|| CoeffVal::scalar(0.0)),
            eq_indices,
        },
        _ => {
            if u_val.is_some() && (h_val.is_some() || r_val.is_some()) {
                return Err(
                    "applyBoundaryCondition: cannot combine 'u' with 'h' or 'r' in a 'mixed' condition"
                        .into(),
                );
            }
            BcKind::Mixed {
                u: u_val,
                h: h_val,
                r: r_val,
                g: g_val,
                q: q_val,
                eq_indices,
            }
        }
    };

    let assignment = BcAssignment { edges, kind };
    let ret_struct = assignment.to_struct(&e.cancel)?;
    let cancel = e.cancel.clone();
    objects::pde_model_update(e, model_id, |model| {
        model.bc_assignments.push(assignment);
        let rec = bc_records_array(&model.bc_assignments, &cancel)?;
        model.boundary_conditions = Some(Arc::new(rec));
        Ok(())
    })?;

    Ok(Value::Container(ret_struct))
}

// ---------------------------------------------------------------------------
// Function-handle evaluation helpers for @(location, state)

pub(crate) fn build_location_struct(
    x: &[f64],
    y: &[f64],
    subdomain: &[f64],
    normal: Option<(&[f64], &[f64])>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Value> {
    let nr = x.len();
    let mut names = vec!["x".to_owned(), "y".to_owned(), "subdomain".to_owned()];
    let mut slots = vec![
        Arc::new(Array::Numeric(Matrix::from_column_slice(1, nr, x)?)),
        Arc::new(Array::Numeric(Matrix::from_column_slice(1, nr, y)?)),
        Arc::new(Array::Numeric(Matrix::from_column_slice(1, nr, subdomain)?)),
    ];
    if let Some((nx, ny)) = normal {
        names.push("nx".to_owned());
        names.push("ny".to_owned());
        slots.push(Arc::new(Array::Numeric(Matrix::from_column_slice(
            1, nr, nx,
        )?)));
        slots.push(Arc::new(Array::Numeric(Matrix::from_column_slice(
            1, nr, ny,
        )?)));
    }
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1, 1, names, slots, cancel,
    )?)))
}

fn build_state_struct(
    n_comp: usize,
    nr: usize,
    time: f64,
    state: Option<(&[f64], &[f64], &[f64])>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Value> {
    let (u_mat, ux_mat, uy_mat) = match state {
        Some((u, ux, uy)) => (
            Matrix::from_column_slice(n_comp, nr, u)?,
            Matrix::from_column_slice(n_comp, nr, ux)?,
            Matrix::from_column_slice(n_comp, nr, uy)?,
        ),
        None => {
            let nans = vec![f64::NAN; n_comp * nr];
            (
                Matrix::from_column_slice(n_comp, nr, &nans)?,
                Matrix::from_column_slice(n_comp, nr, &nans)?,
                Matrix::from_column_slice(n_comp, nr, &nans)?,
            )
        }
    };
    let names = vec![
        "u".to_owned(),
        "ux".to_owned(),
        "uy".to_owned(),
        "time".to_owned(),
    ];
    let slots = vec![
        Arc::new(Array::Numeric(u_mat)),
        Arc::new(Array::Numeric(ux_mat)),
        Arc::new(Array::Numeric(uy_mat)),
        Arc::new(Array::Numeric(Matrix::scalar(time))),
    ];
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1, 1, names, slots, cancel,
    )?)))
}

fn invoke_handle_raw(
    e: &mut Engine,
    out: &mut Execution,
    h: &FunctionHandle,
    loc: Value,
    state: Value,
    label: &str,
) -> Result<Matrix> {
    e.check_cancel()?;
    let evaluated = e.evaluated_elements;
    let res = e.call_handle(h, vec![loc, state], 1, None, out);
    e.evaluated_elements = evaluated;
    let vals = res?;
    let v = vals
        .into_iter()
        .next()
        .ok_or_else(|| format!("solvepde: {label} function handle returned no output"))?;
    let m = match v {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => m,
        Value::Logical(l) | Value::Container(Array::Logical(l)) => {
            let data = l.data().iter().map(|&b| f64::from(b)).collect();
            Matrix::from_column_vec(l.rows(), l.cols(), data)?
        }
        Value::Container(Array::Typed(a)) => a.to_matrix()?,
        _ => {
            return Err(format!(
                "solvepde: {label} function handle must return a real numeric array"
            ));
        }
    };
    if m.is_nd() {
        return Err(format!(
            "solvepde: {label} function handle must return a 2-D numeric array"
        ));
    }
    Ok(m)
}

fn invoke_handle_matrix(
    e: &mut Engine,
    out: &mut Execution,
    h: &FunctionHandle,
    loc: Value,
    state: Value,
    label: &str,
) -> Result<Matrix> {
    let m = invoke_handle_raw(e, out, h, loc, state, label)?;
    if m.data().iter().any(|x| x.is_nan()) {
        return Err(
            "solvepde: nonlinear or state-dependent coefficients and boundary conditions are not supported yet"
                .into(),
        );
    }
    if m.data().iter().any(|x| !x.is_finite()) {
        return Err(format!(
            "solvepde: {label} function handle returned a non-finite value"
        ));
    }
    Ok(m)
}

/// Probe whether a function handle depends on `state.u`, `state.ux`, or `state.uy`.
#[allow(clippy::too_many_arguments)]
fn probe_handle_state_dep(
    e: &mut Engine,
    out: &mut Execution,
    h: &FunctionHandle,
    n_comp: usize,
    x: f64,
    y: f64,
    sub: f64,
    normal: Option<(f64, f64)>,
    label: &str,
) -> Result<bool> {
    let xs = [x];
    let ys = [y];
    let subs = [sub];
    let nxs = normal.map(|(nx, _)| [nx]);
    let nys = normal.map(|(_, ny)| [ny]);
    let n_ref = nxs
        .as_ref()
        .zip(nys.as_ref())
        .map(|(a, b)| (&a[..], &b[..]));

    // 1. Probe with state = NaN.
    let loc_nan = build_location_struct(&xs, &ys, &subs, n_ref, &e.cancel)?;
    let state_nan = build_state_struct(n_comp, 1, f64::NAN, None, &e.cancel)?;
    let m_nan = invoke_handle_raw(e, out, h, loc_nan, state_nan, label)?;
    let has_nan = m_nan.data().iter().any(|v| v.is_nan());

    // 2. Probe with state = 0 (u=0, ux=0, uy=0).
    let zeros = vec![0.0; n_comp];
    let loc0 = build_location_struct(&xs, &ys, &subs, n_ref, &e.cancel)?;
    let state0 = build_state_struct(
        n_comp,
        1,
        f64::NAN,
        Some((&zeros, &zeros, &zeros)),
        &e.cancel,
    )?;
    let m0 = invoke_handle_raw(e, out, h, loc0, state0, label)?;
    if has_nan {
        if m0.data().iter().all(|v| v.is_finite()) {
            return Ok(true);
        }
        return Err(format!(
            "solvepde: {label} function handle returned NaN even for finite state"
        ));
    }

    // 3. Probe with state = 1 (catches piecewise expressions like `state.u > 0`).
    let ones = vec![1.0; n_comp];
    let loc1 = build_location_struct(&xs, &ys, &subs, n_ref, &e.cancel)?;
    let state1 = build_state_struct(n_comp, 1, f64::NAN, Some((&ones, &ones, &ones)), &e.cancel)?;
    let m1 = invoke_handle_raw(e, out, h, loc1, state1, label)?;
    Ok(m0.data() != m1.data())
}

/// Evaluate a vector coefficient (`f`, `g`, `u`, `r`) of component size `n_out` at `xs.len()` points.
/// Returns flat `n_pts * n_out` values (point-major: `vals[pt * n_out + comp]`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn eval_vec_sys_with_state(
    e: &mut Engine,
    out: &mut Execution,
    coeff: &CoeffVal,
    n_out: usize,
    xs: &[f64],
    ys: &[f64],
    subs: &[f64],
    normal: Option<(&[f64], &[f64])>,
    time: f64,
    state: Option<&QuadState>,
    label: &str,
) -> Result<Vec<f64>> {
    let n_pts = xs.len();
    match coeff {
        CoeffVal::Const(v) => {
            let expanded = expand_vec_coeff(v, n_out, label)?;
            let mut result = Vec::with_capacity(n_pts * n_out);
            for _ in 0..n_pts {
                result.extend_from_slice(&expanded);
            }
            Ok(result)
        }
        CoeffVal::Handle(h) => {
            let state_n_comp = state.map_or(n_out, |s| s.n_comp);
            let mut result = Vec::with_capacity(n_pts * n_out);
            let mut start = 0;
            while start < n_pts {
                let end = (start + EVAL_CHUNK).min(n_pts);
                let nr = end - start;
                let loc = build_location_struct(
                    &xs[start..end],
                    &ys[start..end],
                    &subs[start..end],
                    normal.map(|(nx, ny)| (&nx[start..end], &ny[start..end])),
                    &e.cancel,
                )?;
                let st_slice = state.map(|s| {
                    let b0 = start * s.n_comp;
                    let b1 = end * s.n_comp;
                    (&s.u[b0..b1], &s.ux[b0..b1], &s.uy[b0..b1])
                });
                let st = build_state_struct(state_n_comp, nr, time, st_slice, &e.cancel)?;
                let m = invoke_handle_matrix(e, out, h, loc, st, label)?;
                let data = m.data();
                if m.rows() == n_out && m.cols() == nr {
                    result.extend_from_slice(data);
                } else if m.rows() == 1 && m.cols() == nr {
                    for &v in data {
                        for _ in 0..n_out {
                            result.push(v);
                        }
                    }
                } else if m.rows() == 1 && m.cols() == 1 {
                    result.resize(result.len() + nr * n_out, data[0]);
                } else if m.rows() == n_out && m.cols() == 1 {
                    for _ in 0..nr {
                        result.extend_from_slice(data);
                    }
                } else if n_out == 1 && m.rows() == nr && m.cols() == 1 {
                    result.extend_from_slice(data);
                } else {
                    return Err(format!(
                        "solvepde: {label} function handle must return {n_out}-by-{nr} (got {}x{})",
                        m.rows(),
                        m.cols()
                    ));
                }
                start = end;
            }
            Ok(result)
        }
    }
}

/// Evaluate a matrix coefficient (`m`, `d`, `a`, `q`, `h`) of component size `n_out x n_out`
/// at `xs.len()` points. Returns flat `n_pts * n_out * n_out` values (each point has a
/// column-major `n_out x n_out` matrix).
#[allow(clippy::too_many_arguments)]
pub(crate) fn eval_mat_sys_with_state(
    e: &mut Engine,
    out: &mut Execution,
    coeff: &CoeffVal,
    n_out: usize,
    xs: &[f64],
    ys: &[f64],
    subs: &[f64],
    normal: Option<(&[f64], &[f64])>,
    time: f64,
    state: Option<&QuadState>,
    label: &str,
) -> Result<Vec<f64>> {
    let n_pts = xs.len();
    let stride = n_out * n_out;
    match coeff {
        CoeffVal::Const(v) => {
            let expanded = expand_mat_coeff(v, n_out, label)?;
            let mut result = Vec::with_capacity(n_pts * stride);
            for _ in 0..n_pts {
                result.extend_from_slice(&expanded);
            }
            Ok(result)
        }
        CoeffVal::Handle(h) => {
            let state_n_comp = state.map_or(n_out, |s| s.n_comp);
            let mut result = Vec::with_capacity(n_pts * stride);
            let mut start = 0;
            while start < n_pts {
                let end = (start + EVAL_CHUNK).min(n_pts);
                let nr = end - start;
                let loc = build_location_struct(
                    &xs[start..end],
                    &ys[start..end],
                    &subs[start..end],
                    normal.map(|(nx, ny)| (&nx[start..end], &ny[start..end])),
                    &e.cancel,
                )?;
                let st_slice = state.map(|s| {
                    let b0 = start * s.n_comp;
                    let b1 = end * s.n_comp;
                    (&s.u[b0..b1], &s.ux[b0..b1], &s.uy[b0..b1])
                });
                let st = build_state_struct(state_n_comp, nr, time, st_slice, &e.cancel)?;
                let m = invoke_handle_matrix(e, out, h, loc, st, label)?;
                let nc = m.rows();
                let data = m.data();
                if valid_mat_coeff_len(nc, n_out) && m.cols() == nr {
                    for j in 0..nr {
                        let col = &data[j * nc..(j + 1) * nc];
                        result.extend_from_slice(&expand_mat_coeff(col, n_out, label)?);
                    }
                } else if valid_mat_coeff_len(nc, n_out) && m.cols() == 1 {
                    let expanded = expand_mat_coeff(data, n_out, label)?;
                    for _ in 0..nr {
                        result.extend_from_slice(&expanded);
                    }
                } else if m.rows() == n_out && m.cols() == n_out {
                    for _ in 0..nr {
                        result.extend_from_slice(data);
                    }
                } else if n_out == 1 && m.rows() == nr && m.cols() == 1 {
                    result.extend_from_slice(data);
                } else {
                    return Err(format!(
                        "solvepde: {label} function handle returned invalid size {}x{} for {nr} points",
                        m.rows(),
                        m.cols()
                    ));
                }
                start = end;
            }
            Ok(result)
        }
    }
}

/// Evaluate diffusion tensor `c` for an `n_comp`-equation system at `xs.len()` points.
/// Returns flat `n_pts * (4 * n_comp * n_comp)` values (column-major `2N x 2N` per point).
#[allow(clippy::too_many_arguments)]
pub(crate) fn eval_c_sys_with_state(
    e: &mut Engine,
    out: &mut Execution,
    coeff: &CoeffC,
    n_comp: usize,
    xs: &[f64],
    ys: &[f64],
    subs: &[f64],
    time: f64,
    state: Option<&QuadState>,
) -> Result<Vec<f64>> {
    let n_pts = xs.len();
    let stride = 4 * n_comp * n_comp;
    match coeff {
        CoeffC::Const(v) => {
            let expanded = CoeffC::expand_tensor_sys(v, n_comp)?;
            let mut result = Vec::with_capacity(n_pts * stride);
            for _ in 0..n_pts {
                result.extend_from_slice(&expanded);
            }
            Ok(result)
        }
        CoeffC::Handle(h) => {
            let mut result = Vec::with_capacity(n_pts * stride);
            let mut start = 0;
            while start < n_pts {
                let end = (start + EVAL_CHUNK).min(n_pts);
                let nr = end - start;
                let loc = build_location_struct(
                    &xs[start..end],
                    &ys[start..end],
                    &subs[start..end],
                    None,
                    &e.cancel,
                )?;
                let st_slice = state.map(|s| {
                    let b0 = start * s.n_comp;
                    let b1 = end * s.n_comp;
                    (&s.u[b0..b1], &s.ux[b0..b1], &s.uy[b0..b1])
                });
                let st = build_state_struct(n_comp, nr, time, st_slice, &e.cancel)?;
                let m = invoke_handle_matrix(e, out, h, loc, st, "coefficient 'c'")?;
                let nc = m.rows();
                if !valid_c_coeff_len(nc, n_comp) {
                    return Err(format!(
                        "solvepde: coefficient 'c' function handle returned invalid row count {nc} for a {n_comp}-equation system"
                    ));
                }
                let data = m.data();
                if m.cols() == nr {
                    for j in 0..nr {
                        let col = &data[j * nc..(j + 1) * nc];
                        result.extend_from_slice(&CoeffC::expand_tensor_sys(col, n_comp)?);
                    }
                } else if m.cols() == 1 {
                    let expanded = CoeffC::expand_tensor_sys(data, n_comp)?;
                    for _ in 0..nr {
                        result.extend_from_slice(&expanded);
                    }
                } else {
                    return Err(format!(
                        "solvepde: coefficient 'c' function handle must return {nc}-by-{nr} (got {nc}x{})",
                        m.cols()
                    ));
                }
                start = end;
            }
            Ok(result)
        }
    }
}

// ---------------------------------------------------------------------------
// Boundary condition resolution helpers (shared across stationary, time, eigen)

/// Resolve winning `BcAssignment` index per `(edge_0based, comp_0based)`: the last
/// assignment to an edge replaces every earlier one; `None` is the default zero Neumann.
pub(crate) fn resolve_edge_comp_bcs(
    bc_assignments: &[BcAssignment],
    num_edges: usize,
    n_comp: usize,
    who: &str,
) -> Result<Vec<Vec<Option<usize>>>> {
    let mut edge_comp: Vec<Vec<Option<usize>>> = vec![vec![None; n_comp]; num_edges];
    for (idx, bc) in bc_assignments.iter().enumerate() {
        for &eid in &bc.edges {
            if eid < 1 || eid > num_edges {
                return Err(format!(
                    "{who}: boundary condition Edge ID {eid} exceeds geometry NumEdges ({num_edges})"
                ));
            }
            match &bc.kind {
                BcKind::DirichletU { eq_indices, .. }
                | BcKind::DirichletHR { eq_indices, .. }
                | BcKind::Neumann { eq_indices, .. } => match eq_indices {
                    None => {
                        edge_comp[eid - 1].fill(Some(idx));
                    }
                    Some(ids) => {
                        // MATLAB: the last assignment to a region wins; components not listed
                        // in EquationIndex revert to the default zero Neumann condition.
                        edge_comp[eid - 1].fill(None);
                        for &eq in ids {
                            edge_comp[eid - 1][eq - 1] = Some(idx);
                        }
                    }
                },
                BcKind::Mixed { .. } => {
                    edge_comp[eid - 1].fill(Some(idx));
                }
            }
        }
    }
    Ok(edge_comp)
}

/// Evaluate Dirichlet constraints for all active `(bc_idx, edge)` assignments at a given `time`.
/// Returns `(dirichlet_mask, dirichlet_vals)` of length `n_nodes * n_comp` (component-major).
#[allow(clippy::too_many_arguments)]
pub(crate) fn evaluate_dirichlet_sys(
    e: &mut Engine,
    out: &mut Execution,
    mesh: &FeMesh,
    geom: &super::pde_geometry::Geometry2,
    n_comp: usize,
    bc_assignments: &[BcAssignment],
    edge_comp_bc: &[Vec<Option<usize>>],
    time: f64,
    homogeneous_only: bool,
) -> Result<(Vec<bool>, Vec<f64>)> {
    let n_nodes = mesh.nodes.len();
    let n_dofs = n_nodes * n_comp;
    let mut mask = vec![false; n_dofs];
    let mut vals = vec![0.0; n_dofs];
    let num_edges = geom.edges.len();

    for (bc_idx, bc) in bc_assignments.iter().enumerate() {
        // Collect edges where at least one component has this bc_idx as winner.
        let active_edges: Vec<usize> = (1..=num_edges)
            .filter(|&eid| edge_comp_bc[eid - 1].contains(&Some(bc_idx)))
            .collect();
        if active_edges.is_empty() {
            continue;
        }
        if matches!(bc.kind, BcKind::Neumann { .. }) {
            continue;
        }

        let mut node_list: Vec<(usize, usize)> = Vec::new();
        for eid in active_edges {
            for &n1 in &mesh.edge_nodes[eid - 1] {
                node_list.push((n1 - 1, eid));
            }
        }
        if node_list.is_empty() {
            continue;
        }
        let xs: Vec<f64> = node_list.iter().map(|&(ni, _)| mesh.nodes[ni][0]).collect();
        let ys: Vec<f64> = node_list.iter().map(|&(ni, _)| mesh.nodes[ni][1]).collect();
        let subs: Vec<f64> = node_list.iter().map(|&(_, eid)| eid as f64).collect();
        let (nxs, nys) = node_edge_normals(geom, mesh, &node_list);
        let n_pts = node_list.len();

        match &bc.kind {
            BcKind::DirichletU { u, eq_indices } => {
                let comps: Vec<usize> = match eq_indices {
                    Some(ids) => ids.iter().map(|&id| id - 1).collect(),
                    None => (0..n_comp).collect(),
                };
                let m_eq = comps.len();
                let u_vals = if homogeneous_only {
                    vec![0.0; n_pts * m_eq]
                } else {
                    eval_vec_sys_with_state(
                        e,
                        out,
                        u,
                        m_eq,
                        &xs,
                        &ys,
                        &subs,
                        Some((&nxs, &nys)),
                        time,
                        None,
                        "Dirichlet 'u'",
                    )?
                };
                for (pt_i, &(ni, eid)) in node_list.iter().enumerate() {
                    for (loc_c, &comp) in comps.iter().enumerate() {
                        if edge_comp_bc[eid - 1][comp] == Some(bc_idx) {
                            let dof = comp * n_nodes + ni;
                            mask[dof] = true;
                            vals[dof] = u_vals[pt_i * m_eq + loc_c];
                        }
                    }
                }
            }
            BcKind::DirichletHR { h, r, eq_indices } => {
                let comps: Vec<usize> = match eq_indices {
                    Some(ids) => ids.iter().map(|&id| id - 1).collect(),
                    None => (0..n_comp).collect(),
                };
                let m_eq = comps.len();
                if homogeneous_only {
                    for &(ni, eid) in &node_list {
                        for &comp in &comps {
                            if edge_comp_bc[eid - 1][comp] == Some(bc_idx) {
                                let dof = comp * n_nodes + ni;
                                mask[dof] = true;
                                vals[dof] = 0.0;
                            }
                        }
                    }
                } else {
                    let h_vals = eval_mat_sys_with_state(
                        e,
                        out,
                        h,
                        m_eq,
                        &xs,
                        &ys,
                        &subs,
                        Some((&nxs, &nys)),
                        time,
                        None,
                        "Dirichlet 'h'",
                    )?;
                    let r_vals = eval_vec_sys_with_state(
                        e,
                        out,
                        r,
                        m_eq,
                        &xs,
                        &ys,
                        &subs,
                        Some((&nxs, &nys)),
                        time,
                        None,
                        "Dirichlet 'r'",
                    )?;
                    for (pt_i, &(ni, eid)) in node_list.iter().enumerate() {
                        let h_blk = &h_vals[pt_i * m_eq * m_eq..(pt_i + 1) * m_eq * m_eq];
                        let r_blk = &r_vals[pt_i * m_eq..(pt_i + 1) * m_eq];
                        let sol = solve_small_hr(h_blk, r_blk, m_eq)?;
                        for (loc_c, &comp) in comps.iter().enumerate() {
                            if edge_comp_bc[eid - 1][comp] == Some(bc_idx) {
                                let dof = comp * n_nodes + ni;
                                mask[dof] = true;
                                vals[dof] = sol[loc_c];
                            }
                        }
                    }
                }
            }
            BcKind::Mixed {
                u,
                h,
                r,
                eq_indices,
                ..
            } => {
                if let Some(u_c) = u {
                    let comps: Vec<usize> = match eq_indices {
                        Some(ids) => ids.iter().map(|&id| id - 1).collect(),
                        None => (0..n_comp).collect(),
                    };
                    let m_eq = comps.len();
                    let u_vals = if homogeneous_only {
                        vec![0.0; n_pts * m_eq]
                    } else {
                        eval_vec_sys_with_state(
                            e,
                            out,
                            u_c,
                            m_eq,
                            &xs,
                            &ys,
                            &subs,
                            Some((&nxs, &nys)),
                            time,
                            None,
                            "mixed 'u'",
                        )?
                    };
                    for (pt_i, &(ni, eid)) in node_list.iter().enumerate() {
                        for (loc_c, &comp) in comps.iter().enumerate() {
                            if edge_comp_bc[eid - 1][comp] == Some(bc_idx) {
                                let dof = comp * n_nodes + ni;
                                mask[dof] = true;
                                vals[dof] = u_vals[pt_i * m_eq + loc_c];
                            }
                        }
                    }
                } else if let Some(h_c) = h {
                    let h_vals = eval_mat_sys_with_state(
                        e,
                        out,
                        h_c,
                        n_comp,
                        &xs,
                        &ys,
                        &subs,
                        Some((&nxs, &nys)),
                        time,
                        None,
                        "mixed 'h'",
                    )?;
                    let r_vals = if homogeneous_only {
                        vec![0.0; n_pts * n_comp]
                    } else if let Some(r_c) = r {
                        eval_vec_sys_with_state(
                            e,
                            out,
                            r_c,
                            n_comp,
                            &xs,
                            &ys,
                            &subs,
                            Some((&nxs, &nys)),
                            time,
                            None,
                            "mixed 'r'",
                        )?
                    } else {
                        vec![0.0; n_pts * n_comp]
                    };
                    for (pt_i, &(ni, eid)) in node_list.iter().enumerate() {
                        let h_blk = &h_vals[pt_i * n_comp * n_comp..(pt_i + 1) * n_comp * n_comp];
                        let r_blk = &r_vals[pt_i * n_comp..(pt_i + 1) * n_comp];
                        // Identify nonzero rows of H (diagonal or full).
                        let mut nz_rows = Vec::new();
                        for row in 0..n_comp {
                            let row_nz = (0..n_comp).any(|col| h_blk[col * n_comp + row] != 0.0);
                            if row_nz {
                                nz_rows.push(row);
                            }
                        }
                        if nz_rows.is_empty() {
                            continue;
                        }
                        if nz_rows.len() == n_comp {
                            let sol = if homogeneous_only {
                                vec![0.0; n_comp]
                            } else {
                                solve_small_hr(h_blk, r_blk, n_comp)?
                            };
                            for comp in 0..n_comp {
                                if edge_comp_bc[eid - 1][comp] == Some(bc_idx) {
                                    let dof = comp * n_nodes + ni;
                                    mask[dof] = true;
                                    vals[dof] = sol[comp];
                                }
                            }
                        } else {
                            // Extract submatrix on nz_rows (must not couple to unconstrained cols).
                            let m_sub = nz_rows.len();
                            let mut h_sub = vec![0.0; m_sub * m_sub];
                            let mut r_sub = vec![0.0; m_sub];
                            for (ri, &row) in nz_rows.iter().enumerate() {
                                r_sub[ri] = r_blk[row];
                                for col in 0..n_comp {
                                    let v = h_blk[col * n_comp + row];
                                    if let Some(ci) = nz_rows.iter().position(|&c| c == col) {
                                        h_sub[ci * m_sub + ri] = v;
                                    } else if v != 0.0 {
                                        return Err(
                                            "solvepde: mixed boundary condition 'h' couples constrained and unconstrained components"
                                                .into(),
                                        );
                                    }
                                }
                            }
                            let sol = if homogeneous_only {
                                vec![0.0; m_sub]
                            } else {
                                solve_small_hr(&h_sub, &r_sub, m_sub)?
                            };
                            for (ri, &comp) in nz_rows.iter().enumerate() {
                                if edge_comp_bc[eid - 1][comp] == Some(bc_idx) {
                                    let dof = comp * n_nodes + ni;
                                    mask[dof] = true;
                                    vals[dof] = sol[ri];
                                }
                            }
                        }
                    }
                }
            }
            BcKind::Neumann { .. } => {}
        }
    }
    Ok((mask, vals))
}

fn solve_small_hr(h_col_major: &[f64], r: &[f64], m: usize) -> Result<Vec<f64>> {
    if m == 1 {
        if h_col_major[0] == 0.0 {
            return Err("solvepde: Dirichlet coefficient 'h' evaluated to zero".into());
        }
        return Ok(vec![r[0] / h_col_major[0]]);
    }
    let h_mat = DMatrix::from_column_slice(m, m, h_col_major);
    let r_vec = DVector::from_column_slice(r);
    let sol = h_mat
        .lu()
        .solve(&r_vec)
        .ok_or("solvepde: Dirichlet coefficient matrix 'h' is singular")?;
    Ok(sol.as_slice().to_vec())
}

/// Evaluator for Neumann/Robin terms on boundary facets (supports both linear and nonlinear).
pub(crate) struct EdgeNeumannEval {
    pub(crate) n_comp: usize,
    pub(crate) is_state_dep: bool,
    pub(crate) const_q: Vec<f64>,
    pub(crate) const_g: Vec<f64>,
    bc_assignments: Vec<BcAssignment>,
    edge_comp_bc: Vec<Vec<Option<usize>>>,
    facet_edges: Vec<usize>,
    nq_edge: usize,
}

impl EdgeNeumannEval {
    pub(crate) fn eval_at_u(
        &self,
        e: &mut Engine,
        out: &mut Execution,
        mesh: &FeMesh,
        edge_pts: &[EdgeQuadPoint],
        u_nodal: &[f64],
    ) -> Result<(Vec<f64>, Vec<f64>)> {
        // Compute nodal u at edge quadrature points.
        let n_ep = edge_pts.len();
        let nc = self.n_comp;
        let state = QuadState::zeros(nc, n_ep);
        let _ = (mesh, u_nodal);
        eval_neumann_on_edge_pts(
            e,
            out,
            nc,
            &self.bc_assignments,
            &self.edge_comp_bc,
            &self.facet_edges,
            self.nq_edge,
            edge_pts,
            f64::NAN,
            Some(&state),
            false,
        )
    }
}

/// Evaluate `q` and `g` on `edge_pts` for all facets with Neumann or Mixed BCs.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::needless_range_loop)]
pub(crate) fn eval_neumann_on_edge_pts(
    e: &mut Engine,
    out: &mut Execution,
    n_comp: usize,
    bc_assignments: &[BcAssignment],
    edge_comp_bc: &[Vec<Option<usize>>],
    facet_edges: &[usize],
    nq_edge: usize,
    edge_pts: &[EdgeQuadPoint],
    time: f64,
    state: Option<&QuadState>,
    q_only: bool,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let n_ep = edge_pts.len();
    let q_stride = n_comp * n_comp;
    let mut q_vals = vec![0.0; n_ep * q_stride];
    let mut g_vals = vec![0.0; n_ep * n_comp];

    // Group edge quad points by bc_idx that contributes to them.
    let mut bc_groups: Vec<Vec<(usize, usize)>> = vec![Vec::new(); bc_assignments.len()];
    for (fi, &eid) in facet_edges.iter().enumerate() {
        let mut seen = Vec::new();
        for comp in 0..n_comp {
            if let Some(bc_idx) = edge_comp_bc[eid - 1][comp]
                && !seen.contains(&bc_idx)
                && matches!(
                    bc_assignments[bc_idx].kind,
                    BcKind::Neumann { .. } | BcKind::Mixed { .. }
                )
            {
                seen.push(bc_idx);
            }
        }
        for bc_idx in seen {
            for q in 0..nq_edge {
                bc_groups[bc_idx].push((fi * nq_edge + q, eid));
            }
        }
    }

    for (bc_idx, items) in bc_groups.iter().enumerate() {
        if items.is_empty() {
            continue;
        }
        let (g_opt, q_opt, eq_opt) = match &bc_assignments[bc_idx].kind {
            BcKind::Neumann { g, q, eq_indices } => (Some(g), Some(q), eq_indices.as_deref()),
            BcKind::Mixed { g, q, .. } => (g.as_ref(), q.as_ref(), None),
            _ => continue,
        };
        if g_opt.is_none() && q_opt.is_none() {
            continue;
        }
        let comps: Vec<usize> = match eq_opt {
            Some(ids) => ids.iter().map(|&id| id - 1).collect(),
            None => (0..n_comp).collect(),
        };
        let m_eq = comps.len();

        let ep_indices: Vec<usize> = items.iter().map(|&(ep, _)| ep).collect();
        let xs: Vec<f64> = ep_indices.iter().map(|&i| edge_pts[i].x).collect();
        let ys: Vec<f64> = ep_indices.iter().map(|&i| edge_pts[i].y).collect();
        let nxs: Vec<f64> = ep_indices.iter().map(|&i| edge_pts[i].nx).collect();
        let nys: Vec<f64> = ep_indices.iter().map(|&i| edge_pts[i].ny).collect();
        let subs: Vec<f64> = ep_indices
            .iter()
            .map(|&i| edge_pts[i].edge_id as f64)
            .collect();
        let sub_state = state.map(|s| s.slice_indices(&ep_indices));

        if let Some(q_coeff) = q_opt {
            let qv = eval_mat_sys_with_state(
                e,
                out,
                q_coeff,
                m_eq,
                &xs,
                &ys,
                &subs,
                Some((&nxs, &nys)),
                time,
                sub_state.as_ref(),
                "Neumann 'q'",
            )?;
            for (k, &(ep_idx, eid)) in items.iter().enumerate() {
                for (li, &ci) in comps.iter().enumerate() {
                    if edge_comp_bc[eid - 1][ci] == Some(bc_idx) {
                        for (lj, &cj) in comps.iter().enumerate() {
                            q_vals[ep_idx * q_stride + cj * n_comp + ci] =
                                qv[k * m_eq * m_eq + lj * m_eq + li];
                        }
                    }
                }
            }
        }
        if !q_only && let Some(g_coeff) = g_opt {
            let gv = eval_vec_sys_with_state(
                e,
                out,
                g_coeff,
                m_eq,
                &xs,
                &ys,
                &subs,
                Some((&nxs, &nys)),
                time,
                sub_state.as_ref(),
                "Neumann 'g'",
            )?;
            for (k, &(ep_idx, eid)) in items.iter().enumerate() {
                for (li, &ci) in comps.iter().enumerate() {
                    if edge_comp_bc[eid - 1][ci] == Some(bc_idx) {
                        g_vals[ep_idx * n_comp + ci] = gv[k * m_eq + li];
                    }
                }
            }
        }
    }

    Ok((q_vals, g_vals))
}

// ---------------------------------------------------------------------------
// solvepde

fn solve_pde(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    if args.len() == 2 {
        let model_id = objects::pde_model_id(e, &args[0], "solvepde")?;
        return time::solve_pde_time(e, model_id, &args[1], out);
    }
    if args.len() != 1 {
        return Err("solvepde: expected solvepde(model) or solvepde(model, tlist)".into());
    }
    let model_id = objects::pde_model_id(e, &args[0], "solvepde")?;
    let (system_size, analytic, mesh_arr, mesh, coeff_assignments, bc_assignments, solver_options) = {
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
        (
            model.system_size,
            analytic,
            mesh_arr,
            mesh,
            model.coeff_assignments.clone(),
            model.bc_assignments.clone(),
            model.solver_options,
        )
    };

    let n_comp = system_size;
    let num_faces = analytic.geom.num_faces;
    let num_edges = analytic.geom.edges.len();
    let n_nodes = mesh.nodes.len();
    let n_dofs = n_nodes * n_comp;

    // Resolve winning coefficient assignment per 1-based face (later calls override).
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

    // Collect domain quadrature points and group indices by winning CoeffAssignment.
    let tri_face = element_faces(&mesh);
    let (dom_pts, _nq) = collect_domain_quad_points(&mesh, &tri_face, &e.cancel)?;
    let n_dom = dom_pts.len();

    let mut group_indices: Vec<Vec<usize>> = vec![Vec::new(); coeff_assignments.len()];
    for (q_idx, pt) in dom_pts.iter().enumerate() {
        if pt.subdomain < 1 || pt.subdomain > num_faces {
            return Err(format!(
                "solvepde: element has invalid face ID {}",
                pt.subdomain
            ));
        }
        let ca_idx = face_coeff[pt.subdomain - 1].unwrap();
        group_indices[ca_idx].push(q_idx);
    }

    // Probe for state-dependence (nonlinearity) and check m/d == 0.
    let mut is_nonlinear = false;
    for (ca_idx, indices) in group_indices.iter().enumerate() {
        if indices.is_empty() {
            continue;
        }
        let ca = &coeff_assignments[ca_idx];
        let pt0 = dom_pts[indices[0]];
        let xs = [pt0.x];
        let ys = [pt0.y];
        let subs = [pt0.subdomain as f64];
        let zero_st = QuadState::zeros(n_comp, 1);

        // Check m and d at zero state (if handle, also check all points below for linear).
        let mv0 = eval_mat_sys_with_state(
            e,
            out,
            &ca.m,
            n_comp,
            &xs,
            &ys,
            &subs,
            None,
            f64::NAN,
            Some(&zero_st),
            "coefficient 'm'",
        )?;
        let dv0 = eval_mat_sys_with_state(
            e,
            out,
            &ca.d,
            n_comp,
            &xs,
            &ys,
            &subs,
            None,
            f64::NAN,
            Some(&zero_st),
            "coefficient 'd'",
        )?;
        if mv0.iter().any(|&v| v != 0.0) || dv0.iter().any(|&v| v != 0.0) {
            return Err(
                "solvepde: Time-dependent PDE (nonzero 'm' or 'd') requires tlist: call solvepde(model, tlist)."
                    .into(),
            );
        }

        if let CoeffC::Handle(h) = &ca.c
            && probe_handle_state_dep(
                e,
                out,
                h,
                n_comp,
                pt0.x,
                pt0.y,
                pt0.subdomain as f64,
                None,
                "coefficient 'c'",
            )?
        {
            is_nonlinear = true;
        }
        if let CoeffVal::Handle(h) = &ca.a
            && probe_handle_state_dep(
                e,
                out,
                h,
                n_comp,
                pt0.x,
                pt0.y,
                pt0.subdomain as f64,
                None,
                "coefficient 'a'",
            )?
        {
            is_nonlinear = true;
        }
        if let CoeffVal::Handle(h) = &ca.f
            && probe_handle_state_dep(
                e,
                out,
                h,
                n_comp,
                pt0.x,
                pt0.y,
                pt0.subdomain as f64,
                None,
                "coefficient 'f'",
            )?
        {
            is_nonlinear = true;
        }
    }

    // Resolve winning BC assignment per (edge, comp).
    let edge_comp_bc = resolve_edge_comp_bcs(&bc_assignments, num_edges, n_comp, "solvepde")?;

    // Collect Neumann / Mixed facets.
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

    // Evaluate Dirichlet BCs.
    let (dirichlet_mask, dirichlet_vals) = evaluate_dirichlet_sys(
        e,
        out,
        &mesh,
        &analytic.geom,
        n_comp,
        &bc_assignments,
        &edge_comp_bc,
        f64::NAN,
        false,
    )?;

    let u_full = if is_nonlinear {
        let (edge_pts, nq_edge) = if !neumann_facets.is_empty() {
            collect_edge_quad_points(&mesh, &neumann_facets)?
        } else {
            (Vec::new(), 0)
        };
        let (const_q, const_g) = if !neumann_facets.is_empty() {
            eval_neumann_on_edge_pts(
                e,
                out,
                n_comp,
                &bc_assignments,
                &edge_comp_bc,
                &facet_edges,
                nq_edge,
                &edge_pts,
                f64::NAN,
                None,
                false,
            )?
        } else {
            (Vec::new(), Vec::new())
        };
        let neumann_eval = EdgeNeumannEval {
            n_comp,
            is_state_dep: false,
            const_q,
            const_g,
            bc_assignments: bc_assignments.clone(),
            edge_comp_bc: edge_comp_bc.clone(),
            facet_edges,
            nq_edge,
        };
        nonlinear::solve_nonlinear_stationary(
            e,
            out,
            &mesh,
            n_comp,
            &coeff_assignments,
            &dom_pts,
            &group_indices,
            &neumann_facets,
            &edge_pts,
            &neumann_eval,
            &dirichlet_mask,
            &dirichlet_vals,
            solver_options,
        )?
    } else {
        // Linear stationary assembly.
        check_coeff_workspace(n_dom, n_comp)?;
        let c_stride = 4 * n_comp * n_comp;
        let a_stride = n_comp * n_comp;
        let mut c_vals = vec![0.0; n_dom * c_stride];
        let mut a_vals = vec![0.0; n_dom * a_stride];
        let mut f_vals = vec![0.0; n_dom * n_comp];

        for (ca_idx, indices) in group_indices.iter().enumerate() {
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
                f64::NAN,
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
                f64::NAN,
                None,
                "coefficient 'd'",
            )?;
            if mv.iter().any(|&v| v != 0.0) || dv.iter().any(|&v| v != 0.0) {
                return Err(
                    "solvepde: Time-dependent PDE (nonzero 'm' or 'd') requires tlist: call solvepde(model, tlist)."
                        .into(),
                );
            }

            let cv = eval_c_sys_with_state(e, out, &ca.c, n_comp, &xs, &ys, &subs, f64::NAN, None)?;
            let av = eval_mat_sys_with_state(
                e,
                out,
                &ca.a,
                n_comp,
                &xs,
                &ys,
                &subs,
                None,
                f64::NAN,
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
                f64::NAN,
                None,
                "coefficient 'f'",
            )?;

            for (local_k, &q_idx) in indices.iter().enumerate() {
                c_vals[q_idx * c_stride..(q_idx + 1) * c_stride]
                    .copy_from_slice(&cv[local_k * c_stride..(local_k + 1) * c_stride]);
                a_vals[q_idx * a_stride..(q_idx + 1) * a_stride]
                    .copy_from_slice(&av[local_k * a_stride..(local_k + 1) * a_stride]);
                f_vals[q_idx * n_comp..(q_idx + 1) * n_comp]
                    .copy_from_slice(&fv[local_k * n_comp..(local_k + 1) * n_comp]);
            }
        }

        let sys = assemble_kmf_sys(&mesh, n_comp, &c_vals, &a_vals, &f_vals, &e.cancel)?;
        let mut total_a = sys.k.add(&sys.m, 1.0)?;
        let mut total_b = sys.f;

        if !neumann_facets.is_empty() {
            let (edge_pts, nq_edge) = collect_edge_quad_points(&mesh, &neumann_facets)?;
            let (q_vals, g_vals) = eval_neumann_on_edge_pts(
                e,
                out,
                n_comp,
                &bc_assignments,
                &edge_comp_bc,
                &facet_edges,
                nq_edge,
                &edge_pts,
                f64::NAN,
                None,
                false,
            )?;
            let (q_mat, g_vec) =
                assemble_neumann_sys(&mesh, n_comp, &neumann_facets, &q_vals, &g_vals, &e.cancel)?;
            if q_mat.nnz() > 0 {
                total_a = total_a.add(&q_mat, 1.0)?;
            }
            for (bi, &gi) in total_b.iter_mut().zip(g_vec.iter()) {
                *bi += gi;
            }
        }

        let reduced = apply_dirichlet_elimination(
            &total_a,
            &total_b,
            &dirichlet_mask,
            &dirichlet_vals,
            &e.cancel,
        )?;
        let u_free = solve_sparse_system(e, &reduced.a_ii, &reduced.b_eff, out)?;
        reconstruct_solution(
            n_dofs,
            &reduced.free_dofs,
            &u_free,
            &dirichlet_mask,
            &dirichlet_vals,
        )
    };

    let nodal_solution = Matrix::from_column_vec(n_nodes, n_comp, u_full)?;
    let (x_gradients, y_gradients) =
        postproc::compute_nodal_gradients(&mesh, &nodal_solution, &e.cancel)?;
    let results = StationaryResults {
        nodal_solution,
        x_gradients,
        y_gradients,
        mesh,
        mesh_arr,
    };
    objects::pde_stationary_results_new(e, Arc::new(results))
}

/// Approximate outward unit normal at boundary nodes along their geometry edge.
pub(crate) fn node_edge_normals(
    geom: &super::pde_geometry::Geometry2,
    mesh: &FeMesh,
    node_list: &[(usize, usize)],
) -> (Vec<f64>, Vec<f64>) {
    let mut nxs = Vec::with_capacity(node_list.len());
    let mut nys = Vec::with_capacity(node_list.len());
    for &(ni, eid) in node_list {
        let p = mesh.nodes[ni];
        let edge = &geom.edges[eid - 1];
        let mut best_s = 0.5;
        let mut best_d2 = f64::INFINITY;
        for k in 0..=32 {
            let s = k as f64 / 32.0;
            let q = edge.point_at(s);
            let d2 = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
            if d2 < best_d2 {
                best_d2 = d2;
                best_s = s;
            }
        }
        let s0 = (best_s - 1e-4).max(0.0);
        let s1 = (best_s + 1e-4).min(1.0);
        let a = edge.point_at(s0);
        let b = edge.point_at(s1);
        let tx = b[0] - a[0];
        let ty = b[1] - a[1];
        let len = tx.hypot(ty);
        if len > 0.0 {
            let sign = if edge.left == 0 && edge.right > 0 {
                -1.0
            } else {
                1.0
            };
            nxs.push(sign * ty / len);
            nys.push(-sign * tx / len);
        } else {
            nxs.push(0.0);
            nys.push(0.0);
        }
    }
    (nxs, nys)
}
