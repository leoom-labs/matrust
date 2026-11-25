//! 2-D PDE eigenvalue solver (`solvepdeeig`) and results object (`pde.EigenResults`).
//!
//! Solves the generalized symmetric eigenvalue problem `K v = lambda M v` for all
//! eigenvalues `lambda` in `[lo, hi]` (`lo` may be `-Inf`), where:
//! - `K` is assembled from `c`, `a`, and generalized Neumann `q`
//! - `M` is assembled from `d` (symmetric positive definite)
//! - Homogeneous Dirichlet boundary conditions eliminate constrained DOFs (padded with 0).
//!
//! Uses a dense generalized symmetric eigensolver for small problems (`n <= DENSE_THRESHOLD`)
//! and spectrum-sliced shift-invert Lanczos with sparse LDLT/LU factors (`faer`) for larger
//! problems, with a dense fallback up to `DENSE_FALLBACK_MAX`.
use super::assemble::{
    BoundaryFacet, apply_dirichlet_elimination, assemble_kmf_sys, assemble_mass_sys,
    assemble_neumann_sys, collect_domain_quad_points, collect_edge_quad_points, element_faces,
    extract_boundary_facets,
};
use super::types::{BcKind, text_value};
use super::{
    check_coeff_workspace, eval_c_sys_with_state, eval_mat_sys_with_state,
    eval_neumann_on_edge_pts, evaluate_dirichlet_sys, format_dims, resolve_edge_comp_bcs,
};
use crate::engine::sparse_solve::{check_lu_fill, faer_ref};
use crate::engine::{Engine, Execution, Value, objects, pde_mesh::FeMesh};
use crate::sparse::{MAX_NNZ, SparseMatrix, SparseValues};
use crate::{Array, Matrix, Result, StructArray};
use faer::Side;
use faer::dyn_stack::{MemBuffer, MemStack};
use faer::linalg::cholesky::ldlt::factor::LdltRegularization;
use faer::linalg::solvers::Solve;
use faer::prelude::*;
use faer::sparse::linalg::LuError;
use faer::sparse::linalg::cholesky::{
    LdltRef, SymbolicCholesky, SymbolicCholeskyRaw, factorize_symbolic_cholesky, supernodal,
};
use faer::sparse::linalg::solvers::Lu;
use faer::{Conj, Par};
use nalgebra::{DMatrix, SymmetricEigen};
use std::collections::VecDeque;
use std::sync::Arc;

pub(crate) const EIGEN_RESULTS_CLASS: &str = "pde.EigenResults";
/// MATLAB `pde.EigenResults` property order: Eigenvectors, Eigenvalues, Mesh.
pub(crate) const EIGEN_RESULTS_PROPERTIES: [&str; 3] = ["Eigenvectors", "Eigenvalues", "Mesh"];

/// Direct dense generalized symmetric eigensolve threshold.
const DENSE_THRESHOLD: usize = 250;
/// Maximum reduced size eligible for dense fallback if sparse shift-invert fails.
const DENSE_FALLBACK_MAX: usize = 2000;
/// Maximum number of distinct shifts explored by spectrum-sliced shift-invert Lanczos.
const MAX_SHIFTS: usize = 64;
/// Relative residual tolerance for accepting a converged generalized eigenpair.
const EIG_RES_TOL: f64 = 1e-10;
/// Maximum eigenvalues collected before refusing unbounded output.
const MAX_EIGENPAIRS: usize = 2000;

// ---------------------------------------------------------------------------
// pde.EigenResults (value semantics)

#[derive(Clone, Debug)]
pub(crate) struct EigenResults {
    pub(crate) eigenvalues: Matrix,
    pub(crate) eigenvectors: Matrix,
    pub(crate) mesh: Arc<FeMesh>,
    pub(crate) mesh_arr: Arc<Array>,
    /// Equation count. Eigenvectors of shape `N x Ncomp x 1` collapse to `N x Ncomp`,
    /// so the component count cannot be recovered from the matrix dimensions alone.
    pub(crate) n_comp: usize,
}

impl PartialEq for EigenResults {
    fn eq(&self, other: &Self) -> bool {
        self.eigenvalues.rows() == other.eigenvalues.rows()
            && self.eigenvalues.cols() == other.eigenvalues.cols()
            && self.eigenvalues.data() == other.eigenvalues.data()
            && self.eigenvectors.rows() == other.eigenvectors.rows()
            && self.eigenvectors.cols() == other.eigenvectors.cols()
            && self.eigenvectors.data() == other.eigenvectors.data()
            && self.mesh == other.mesh
            && self.n_comp == other.n_comp
    }
}

impl EigenResults {
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + (self.eigenvalues.len() + self.eigenvectors.len()) * std::mem::size_of::<f64>()
            + self.mesh_arr.retained_bytes()
    }

    pub(crate) fn push_children<'a>(&'a self, pending: &mut Vec<&'a Array>) {
        pending.push(self.mesh_arr.as_ref());
    }

    pub(crate) fn property(&self, name: &str) -> Result<Value> {
        match name {
            "Eigenvalues" => Ok(Value::Matrix(self.eigenvalues.clone())),
            "Eigenvectors" => Ok(Value::Matrix(self.eigenvectors.clone())),
            "Mesh" => Ok(Value::Container(self.mesh_arr.as_ref().clone())),
            _ => Err(format!(
                "No appropriate method, property, or field '{name}' for class '{EIGEN_RESULTS_CLASS}'."
            )),
        }
    }

    pub(crate) fn property_ci(&self, name: &str) -> Result<Value> {
        if let Some(&canonical) = EIGEN_RESULTS_PROPERTIES
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
                let mut fields = Vec::with_capacity(EIGEN_RESULTS_PROPERTIES.len());
                for &prop in &EIGEN_RESULTS_PROPERTIES {
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
                    text_value(prop).ok_or("pde.EigenResults: property name must be text")?;
                self.property_ci(&name)
            }
            _ => Err("pde.EigenResults: get expects zero or one property name".into()),
        }
    }

    pub(crate) fn body(&self) -> String {
        format!(
            "  EigenResults with properties:\n\n\
             {:>18}: [{} double]\n\
             {:>18}: [{}x{} double]\n\
             {:>18}: [1x1 FEMesh]\n",
            "Eigenvectors",
            format_dims(&self.eigenvectors),
            "Eigenvalues",
            self.eigenvalues.rows(),
            self.eigenvalues.cols(),
            "Mesh",
        )
    }
}

// ---------------------------------------------------------------------------
// Argument parsing & entry point

fn parse_eigenvalue_range(v: &Value) -> Result<(f64, f64)> {
    let m = match v {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => m.clone(),
        Value::Logical(l) | Value::Container(Array::Logical(l)) => {
            let data = l.data().iter().map(|&b| f64::from(b)).collect();
            Matrix::from_column_vec(l.rows(), l.cols(), data)?
        }
        Value::Container(Array::Typed(a)) => a.to_matrix()?,
        _ => {
            return Err(
                "solvepdeeig: Eigenvalue range must be a two-element real numeric vector [lo, hi]."
                    .into(),
            );
        }
    };
    if m.is_nd() || m.len() != 2 || (m.rows() != 1 && m.cols() != 1) {
        return Err(
            "solvepdeeig: Eigenvalue range must be a two-element real vector [lo, hi].".into(),
        );
    }
    let lo = m.data()[0];
    let hi = m.data()[1];
    if lo.is_nan() || hi.is_nan() {
        return Err("solvepdeeig: Eigenvalue range must not contain NaN.".into());
    }
    if lo == f64::INFINITY || !hi.is_finite() {
        return Err(
            "solvepdeeig: Upper bound of eigenvalue range must be finite, and lower bound must be finite or -Inf."
                .into(),
        );
    }
    if lo > hi {
        return Err("solvepdeeig: Eigenvalue range [lo, hi] must satisfy lo <= hi.".into());
    }
    Ok((lo, hi))
}

pub(crate) fn solve_pde_eig(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    if args.len() < 2 {
        return Err(
            "solvepdeeig: Not enough input arguments. Expected solvepdeeig(model, evr).".into(),
        );
    }
    if args.len() > 2 {
        return Err(
            "solvepdeeig: Too many input arguments. Expected solvepdeeig(model, evr).".into(),
        );
    }
    let model_id = objects::pde_model_id(e, &args[0], "solvepdeeig")?;
    let (lo, hi) = parse_eigenvalue_range(&args[1])?;

    let (system_size, analytic, mesh_arr, mesh, coeff_assignments, bc_assignments) = {
        let model = objects::pde_model_get(e, model_id)
            .ok_or("solvepdeeig: invalid or deleted pde.PDEModel")?;
        let analytic = model.geometry.as_ref().map(|(g, _)| g.clone()).ok_or(
            "solvepdeeig: The model has no geometry. Attach a geometry using geometryFromEdges.",
        )?;
        let mesh_arr = model
            .mesh
            .as_ref()
            .cloned()
            .ok_or("solvepdeeig: The model has no mesh. Generate a mesh using generateMesh.")?;
        let mesh_val = Value::Container(mesh_arr.as_ref().clone());
        let mesh: Arc<FeMesh> = objects::pde_mesh_value(e, &mesh_val)
            .ok_or("solvepdeeig: model.Mesh is not a valid pde.FEMesh")?;
        if model.coeff_assignments.is_empty() {
            return Err(
                "solvepdeeig: The model has no equation coefficients. Specify coefficients using specifyCoefficients."
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
        )
    };

    let n_comp = system_size;
    let num_faces = analytic.geom.num_faces;
    let num_edges = analytic.geom.edges.len();
    let n_nodes = mesh.nodes.len();
    let n_dofs = n_nodes * n_comp;

    // Resolve winning coefficient assignment per 1-based face.
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
                            "solvepdeeig: coefficient Face ID {id} exceeds geometry NumFaces ({num_faces})"
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
                "solvepdeeig: no equation coefficients specified for face {}",
                fi + 1
            ));
        }
    }

    // Collect domain quadrature points and evaluate c, a, m, d.
    let tri_face = element_faces(&mesh);
    let (dom_pts, _nq) = collect_domain_quad_points(&mesh, &tri_face, &e.cancel)?;
    let n_dom = dom_pts.len();

    let mut group_indices: Vec<Vec<usize>> = vec![Vec::new(); coeff_assignments.len()];
    for (q_idx, pt) in dom_pts.iter().enumerate() {
        if pt.subdomain < 1 || pt.subdomain > num_faces {
            return Err(format!(
                "solvepdeeig: element has invalid face ID {}",
                pt.subdomain
            ));
        }
        let ca_idx = face_coeff[pt.subdomain - 1].unwrap();
        group_indices[ca_idx].push(q_idx);
    }

    check_coeff_workspace(n_dom, n_comp)?;
    let c_stride = 4 * n_comp * n_comp;
    let m_stride = n_comp * n_comp;
    let mut c_vals = vec![0.0; n_dom * c_stride];
    let mut a_vals = vec![0.0; n_dom * m_stride];
    let mut d_vals = vec![0.0; n_dom * m_stride];
    let f_zeros = vec![0.0; n_dom * n_comp];

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
        if mv.iter().any(|&v| v != 0.0) {
            return Err(
                "solvepdeeig: Nonzero coefficient 'm' is not supported; specify m = 0 and positive d."
                    .into(),
            );
        }
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
        ensure_symmetric_blocks(&dv, n_comp, "d")?;
        for blk in dv.chunks_exact(m_stride) {
            for i in 0..n_comp {
                if blk[i * n_comp + i] <= 0.0 {
                    return Err(
                        "solvepdeeig: Coefficient 'd' must be positive for eigenvalue problems."
                            .into(),
                    );
                }
            }
        }
        let cv = eval_c_sys_with_state(e, out, &ca.c, n_comp, &xs, &ys, &subs, f64::NAN, None)?;
        let m2 = 2 * n_comp;
        for blk in cv.chunks_exact(c_stride) {
            let scale = blk.iter().fold(1.0f64, |acc, &v| acc.max(v.abs()));
            for col in 0..m2 {
                for row in (col + 1)..m2 {
                    if (blk[col * m2 + row] - blk[row * m2 + col]).abs() > 1e-12 * scale {
                        return Err(
                            "solvepdeeig: Nonsymmetric diffusion coefficient 'c' is not supported."
                                .into(),
                        );
                    }
                }
            }
        }
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
        ensure_symmetric_blocks(&av, n_comp, "a")?;

        for (local_k, &q_idx) in indices.iter().enumerate() {
            c_vals[q_idx * c_stride..(q_idx + 1) * c_stride]
                .copy_from_slice(&cv[local_k * c_stride..(local_k + 1) * c_stride]);
            a_vals[q_idx * m_stride..(q_idx + 1) * m_stride]
                .copy_from_slice(&av[local_k * m_stride..(local_k + 1) * m_stride]);
            d_vals[q_idx * m_stride..(q_idx + 1) * m_stride]
                .copy_from_slice(&dv[local_k * m_stride..(local_k + 1) * m_stride]);
        }
    }

    // Assemble domain K (from c and a) and M (from d).
    let sys = assemble_kmf_sys(&mesh, n_comp, &c_vals, &a_vals, &f_zeros, &e.cancel)?;
    let mut total_k = sys.k.add(&sys.m, 1.0)?;
    let total_m = assemble_mass_sys(&mesh, n_comp, &d_vals, &e.cancel)?;

    // Resolve winning BC assignment per (edge, comp).
    let edge_comp_bc = resolve_edge_comp_bcs(&bc_assignments, num_edges, n_comp, "solvepdeeig")?;

    // Assemble Robin 'q' terms on active Neumann/Mixed edges (g is ignored for eigenproblems).
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
    if !neumann_facets.is_empty() {
        let (edge_pts, nq_edge) = collect_edge_quad_points(&mesh, &neumann_facets)?;
        let (q_vals, g_zeros) = eval_neumann_on_edge_pts(
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
            true,
        )?;
        ensure_symmetric_blocks(&q_vals, n_comp, "q")?;
        let (q_mat, _) =
            assemble_neumann_sys(&mesh, n_comp, &neumann_facets, &q_vals, &g_zeros, &e.cancel)?;
        if q_mat.nnz() > 0 {
            total_k = total_k.add(&q_mat, 1.0)?;
        }
    }

    // Identify homogeneous Dirichlet DOFs.
    let (dirichlet_mask, _) = evaluate_dirichlet_sys(
        e,
        out,
        &mesh,
        &analytic.geom,
        n_comp,
        &bc_assignments,
        &edge_comp_bc,
        f64::NAN,
        true,
    )?;

    let zeros_n = vec![0.0; n_dofs];
    let k_red =
        apply_dirichlet_elimination(&total_k, &zeros_n, &dirichlet_mask, &zeros_n, &e.cancel)?;
    let m_red =
        apply_dirichlet_elimination(&total_m, &zeros_n, &dirichlet_mask, &zeros_n, &e.cancel)?;
    let free_dofs = k_red.free_dofs;
    let n_free = free_dofs.len();

    let pairs = if n_free == 0 {
        Vec::new()
    } else if n_free <= DENSE_THRESHOLD {
        solve_dense_gen_sym(&k_red.a_ii, &m_red.a_ii, lo, hi, e)?
    } else {
        match solve_sparse_shift_invert(&k_red.a_ii, &m_red.a_ii, lo, hi, e) {
            Ok(p) => p,
            Err(err) if n_free <= DENSE_FALLBACK_MAX => {
                if err.contains("cancelled") || err.contains("positive definite") {
                    return Err(err);
                }
                solve_dense_gen_sym(&k_red.a_ii, &m_red.a_ii, lo, hi, e)?
            }
            Err(err) => return Err(err),
        }
    };

    let k_found = pairs.len();
    let mut evals = Vec::with_capacity(k_found);
    let mut evecs = vec![0.0; n_dofs * k_found];
    for (col, (lam, v_free)) in pairs.into_iter().enumerate() {
        evals.push(lam);
        let col_off = col * n_dofs;
        for (&dof, &val) in free_dofs.iter().zip(v_free.iter()) {
            evecs[col_off + dof] = val;
        }
    }

    let eigenvalues = Matrix::from_column_vec(k_found, 1, evals)?;
    let eigenvectors = if n_comp == 1 {
        Matrix::from_column_vec(n_nodes, k_found, evecs)?
    } else {
        Matrix::from_column_vec(n_nodes, n_comp * k_found, evecs)?
            .with_dims(&[n_nodes, n_comp, k_found])?
    };
    let results = EigenResults {
        eigenvalues,
        eigenvectors,
        mesh,
        mesh_arr,
        n_comp,
    };
    objects::pde_eigen_results_new(e, Arc::new(results))
}

/// Reject a column-major `n x n` block that is not symmetric. Scalar (`n == 1`)
/// blocks and the symmetric `N(N+1)/2` expansion are accepted. Tolerance matches
/// the diffusion-tensor check on `c`.
fn ensure_symmetric_blocks(vals: &[f64], n: usize, coeff: &str) -> Result<()> {
    if n <= 1 {
        return Ok(());
    }
    let stride = n * n;
    for blk in vals.chunks_exact(stride) {
        let scale = blk.iter().fold(1.0_f64, |acc, &v| acc.max(v.abs()));
        for col in 0..n {
            for row in (col + 1)..n {
                if (blk[col * n + row] - blk[row * n + col]).abs() > 1e-12 * scale {
                    return Err(format!(
                        "solvepdeeig: Nonsymmetric coefficient '{coeff}' is not supported."
                    ));
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Dense generalized symmetric eigensolver (small n / fallback)

fn solve_dense_gen_sym(
    k: &SparseMatrix,
    m: &SparseMatrix,
    lo: f64,
    hi: f64,
    e: &Engine,
) -> Result<Vec<(f64, Vec<f64>)>> {
    let n = k.rows();
    e.check_cancel()?;
    let k_vals = k.dense_values(&e.cancel)?;
    let m_vals = m.dense_values(&e.cancel)?;

    let mut k_mat = DMatrix::from_column_slice(n, n, &k_vals);
    let mut m_mat = DMatrix::from_column_slice(n, n, &m_vals);
    // Enforce exact symmetry before Cholesky / SymmetricEigen.
    for j in 0..n {
        for i in (j + 1)..n {
            let ks = 0.5 * (k_mat[(i, j)] + k_mat[(j, i)]);
            k_mat[(i, j)] = ks;
            k_mat[(j, i)] = ks;
            let ms = 0.5 * (m_mat[(i, j)] + m_mat[(j, i)]);
            m_mat[(i, j)] = ms;
            m_mat[(j, i)] = ms;
        }
    }

    let chol = m_mat
        .cholesky()
        .ok_or("solvepdeeig: Mass matrix is not symmetric positive definite.")?;
    let l = chol.l();
    e.check_cancel()?;

    // Form C = L^{-1} K L^{-T}:
    // 1. Solve L Y = K
    let y = l
        .solve_lower_triangular(&k_mat)
        .ok_or("solvepdeeig: Cholesky triangular solve failed")?;
    // 2. Solve L Z = Y^T  =>  Z = L^{-1} K^T L^{-T} = C
    let mut c = l
        .solve_lower_triangular(&y.transpose())
        .ok_or("solvepdeeig: Cholesky triangular solve failed")?;
    for j in 0..n {
        for i in (j + 1)..n {
            let s = 0.5 * (c[(i, j)] + c[(j, i)]);
            c[(i, j)] = s;
            c[(j, i)] = s;
        }
    }
    e.check_cancel()?;

    let eig = SymmetricEigen::new(c);
    let mut pairs: Vec<(f64, usize)> = eig
        .eigenvalues
        .iter()
        .enumerate()
        .filter_map(|(idx, &lam)| (lam >= lo && lam <= hi).then_some((lam, idx)))
        .collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

    if pairs.is_empty() {
        return Ok(Vec::new());
    }

    // Back-transform selected eigenvectors: solve L^T V = Q_sel.
    let k_sel = pairs.len();
    let mut q_sel = DMatrix::zeros(n, k_sel);
    for (col, &(_, idx)) in pairs.iter().enumerate() {
        q_sel.set_column(col, &eig.eigenvectors.column(idx));
    }
    let lt = l.transpose();
    let v_sel = lt
        .solve_upper_triangular(&q_sel)
        .ok_or("solvepdeeig: Cholesky back-substitution failed")?;

    let mut out = Vec::with_capacity(k_sel);
    for (col, &(lam, _)) in pairs.iter().enumerate() {
        let mut v: Vec<f64> = v_sel.column(col).iter().copied().collect();
        // Ensure exact M-unit norm.
        let mv = spmv(m, &v);
        let norm_m = dot(&v, &mv).max(0.0).sqrt();
        if norm_m > 0.0 {
            for x in &mut v {
                *x /= norm_m;
            }
        }
        out.push((lam, v));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Sparse linear algebra helpers

fn doubles(s: &SparseMatrix) -> &[f64] {
    match s.values() {
        SparseValues::Double(v) => v,
        SparseValues::Logical(_) => unreachable!("FEM matrices are always double"),
    }
}

/// Sparse matrix-vector product `y = A * x` for CSC `SparseMatrix`.
fn spmv(a: &SparseMatrix, x: &[f64]) -> Vec<f64> {
    let n = a.rows();
    let rows = a.rowidx();
    let vals = doubles(a);
    let mut y = vec![0.0; n];
    for (c, &xc) in x.iter().enumerate() {
        if xc == 0.0 {
            continue;
        }
        for k in a.column(c) {
            y[rows[k]] += vals[k] * xc;
        }
    }
    y
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum()
}

fn norm2(a: &[f64]) -> f64 {
    dot(a, a).sqrt()
}

/// Matrix 1-norm (maximum absolute column sum) of a CSC sparse matrix.
fn norm1(a: &SparseMatrix) -> f64 {
    let vals = doubles(a);
    let mut max_col: f64 = 0.0;
    for c in 0..a.cols() {
        let mut sum = 0.0;
        for k in a.column(c) {
            sum += vals[k].abs();
        }
        max_col = max_col.max(sum);
    }
    max_col
}

/// Verify that sparse symmetric `M` is SPD via `faer` sparse Cholesky.
fn verify_sparse_spd(m: &SparseMatrix, e: &mut Engine) -> Result<()> {
    let vals = doubles(m);
    let a = faer_ref(m, vals);
    let symbolic = factorize_symbolic_cholesky(
        a.symbolic(),
        Side::Lower,
        Default::default(),
        Default::default(),
    )
    .map_err(|err| format!("solvepdeeig: sparse Cholesky analysis of M failed ({err:?})"))?;
    let factor_nnz = symbolic.len_val();
    if factor_nnz as u64 > MAX_NNZ {
        return Err(format!(
            "solvepdeeig: sparse Cholesky factor of M needs {factor_nnz} nonzeros, more than {MAX_NNZ}"
        ));
    }
    e.charge(factor_nnz)?;
    e.check_cancel()?;
    let mut l = vec![0.0; factor_nnz];
    let mut mem = MemBuffer::try_new(
        symbolic.factorize_numeric_llt_scratch::<f64>(Par::Seq, Default::default()),
    )
    .map_err(|_| "solvepdeeig: out of memory for sparse Cholesky workspace".to_string())?;
    symbolic
        .factorize_numeric_llt(
            &mut l,
            a,
            Side::Lower,
            Default::default(),
            Par::Seq,
            MemStack::new(&mut mem),
            Default::default(),
        )
        .map_err(|_| "solvepdeeig: Mass matrix is not symmetric positive definite.".to_string())?;
    Ok(())
}

/// Factorized shifted operator `A_sigma = K - sigma * M`.
enum ShiftedFactor {
    Ldlt {
        symbolic: SymbolicCholesky<usize>,
        values: Vec<f64>,
        inertia: usize,
    },
    Lu(Lu<usize, f64>),
}

impl ShiftedFactor {
    fn inertia(&self) -> Option<usize> {
        match self {
            Self::Ldlt { inertia, .. } => Some(*inertia),
            Self::Lu(_) => None,
        }
    }

    fn solve(&self, rhs: &[f64]) -> Result<Vec<f64>> {
        let n = rhs.len();
        let mut x = rhs.to_vec();
        match self {
            Self::Ldlt {
                symbolic, values, ..
            } => {
                let m = MatMut::from_column_major_slice_mut(&mut x, n, 1);
                let mut mem = MemBuffer::new(symbolic.solve_in_place_scratch::<f64>(1, Par::Seq));
                LdltRef::new(symbolic, values).solve_in_place_with_conj(
                    Conj::No,
                    m,
                    Par::Seq,
                    MemStack::new(&mut mem),
                );
            }
            Self::Lu(lu) => {
                let m = MatMut::from_column_major_slice_mut(&mut x, n, 1);
                lu.solve_in_place(m);
            }
        }
        if x.iter().any(|v| !v.is_finite()) {
            return Err("solvepdeeig: shifted linear solve produced non-finite values".into());
        }
        Ok(x)
    }
}

/// Extract the diagonal pivots of `faer`'s sparse LDLT factor and count negative pivots
/// (= exact number of eigenvalues of `(K, M)` strictly less than `sigma` by Sylvester's law).
/// Returns `None` if any pivot is non-finite or too close to zero for reliable inertia/solve.
fn ldlt_inertia_checked(symbolic: &SymbolicCholesky<usize>, values: &[f64]) -> Option<usize> {
    let n = symbolic.nrows();
    let mut max_abs = 0.0f64;
    let mut min_abs = f64::INFINITY;
    let mut neg_count = 0usize;

    match symbolic.raw() {
        SymbolicCholeskyRaw::Simplicial(sym) => {
            let col_ptr = sym.col_ptr();
            for j in 0..n {
                let d = values[col_ptr[j]];
                if !d.is_finite() {
                    return None;
                }
                let ad = d.abs();
                max_abs = max_abs.max(ad);
                min_abs = min_abs.min(ad);
                if d < 0.0 {
                    neg_count += 1;
                }
            }
        }
        SymbolicCholeskyRaw::Supernodal(sym) => {
            let ldlt = supernodal::SupernodalLdltRef::new(sym, values);
            for s in 0..sym.n_supernodes() {
                let sn = ldlt.supernode(s);
                let val = sn.val();
                let ncols = val.ncols();
                for k in 0..ncols {
                    let d = val[(k, k)];
                    if !d.is_finite() {
                        return None;
                    }
                    let ad = d.abs();
                    max_abs = max_abs.max(ad);
                    min_abs = min_abs.min(ad);
                    if d < 0.0 {
                        neg_count += 1;
                    }
                }
            }
        }
    }

    if max_abs == 0.0 || min_abs <= 1e-13 * max_abs {
        return None;
    }
    Some(neg_count)
}

/// Factorize `K - sigma * M`, preferring sparse LDLT (for exact Sylvester inertia)
/// and falling back to sparse LU if LDLT hits a small/zero pivot.
fn factorize_shifted(
    k: &SparseMatrix,
    m: &SparseMatrix,
    sigma: f64,
    e: &mut Engine,
) -> Result<ShiftedFactor> {
    e.check_cancel()?;
    let a_mat = k.add(m, -sigma)?;
    let vals = doubles(&a_mat);
    let a_ref = faer_ref(&a_mat, vals);

    // 1. Try sparse LDLT (gives exact inertia when well-conditioned).
    if let Ok(symbolic) = factorize_symbolic_cholesky(
        a_ref.symbolic(),
        Side::Lower,
        Default::default(),
        Default::default(),
    ) {
        let factor_nnz = symbolic.len_val();
        if factor_nnz as u64 <= MAX_NNZ {
            e.charge(factor_nnz)?;
            e.check_cancel()?;
            let mut l_vals = Vec::new();
            if l_vals.try_reserve_exact(factor_nnz).is_ok() {
                l_vals.resize(factor_nnz, 0.0);
                if let Ok(mut mem) = MemBuffer::try_new(
                    symbolic.factorize_numeric_ldlt_scratch::<f64>(Par::Seq, Default::default()),
                ) && symbolic
                    .factorize_numeric_ldlt(
                        &mut l_vals,
                        a_ref,
                        Side::Lower,
                        LdltRegularization::default(),
                        Par::Seq,
                        MemStack::new(&mut mem),
                        Default::default(),
                    )
                    .is_ok()
                    && let Some(inertia) = ldlt_inertia_checked(&symbolic, &l_vals)
                {
                    return Ok(ShiftedFactor::Ldlt {
                        symbolic,
                        values: l_vals,
                        inertia,
                    });
                }
            }
        }
    }

    // 2. Fallback to sparse LU.
    e.check_cancel()?;
    let bound = check_lu_fill(e, "solvepdeeig", &a_mat)?;
    e.charge(bound as usize)?;
    match a_ref.sp_lu() {
        Ok(lu) => Ok(ShiftedFactor::Lu(lu)),
        Err(LuError::SymbolicSingular { .. }) => {
            Err("solvepdeeig: shifted matrix K - sigma*M is singular".into())
        }
        Err(LuError::Generic(err)) => Err(format!(
            "solvepdeeig: sparse LU factorization failed ({err:?})"
        )),
    }
}

/// Factorize at `sigma`, perturbing slightly if `sigma` lands on an eigenvalue.
fn factorize_shifted_retry(
    k: &SparseMatrix,
    m: &SparseMatrix,
    sigma: f64,
    e: &mut Engine,
) -> Result<(f64, ShiftedFactor)> {
    let scale = sigma.abs().max(1.0);
    let offsets = [0.0, 1e-7, -1e-7, 1e-5, -1e-5, 1e-4];
    let mut last_err = String::new();
    for &off in &offsets {
        let s = sigma + off * scale;
        match factorize_shifted(k, m, s, e) {
            Ok(fac) => return Ok((s, fac)),
            Err(err) => last_err = err,
        }
    }
    Err(last_err)
}

// ---------------------------------------------------------------------------
// Spectrum-sliced deflated shift-invert Lanczos

struct LockedSpace {
    evals: Vec<f64>,
    /// M-orthonormal eigenvectors (each length n).
    vecs: Vec<Vec<f64>>,
    /// Precomputed `M * v` for each locked eigenvector.
    m_vecs: Vec<Vec<f64>>,
}

impl LockedSpace {
    fn new() -> Self {
        Self {
            evals: Vec::new(),
            vecs: Vec::new(),
            m_vecs: Vec::new(),
        }
    }

    fn len(&self) -> usize {
        self.evals.len()
    }

    fn count_in(&self, a: f64, b: f64) -> usize {
        self.evals
            .iter()
            .filter(|&&lam| lam >= a && lam <= b)
            .count()
    }

    /// Two-pass CGS M-orthogonalization of `z` against all locked eigenvectors.
    fn deflate(&self, z: &mut [f64]) {
        for _ in 0..2 {
            for (v, mv) in self.vecs.iter().zip(self.m_vecs.iter()) {
                let h = dot(z, mv);
                if h != 0.0 {
                    for (zi, &vi) in z.iter_mut().zip(v.iter()) {
                        *zi -= h * vi;
                    }
                }
            }
        }
    }

    /// Try to lock a converged eigenpair `(lam, v)` after M-orthogonalizing against
    /// existing locked vectors. Returns `true` if accepted (independent of locked space).
    fn try_add(&mut self, lam: f64, mut v: Vec<f64>, m: &SparseMatrix) -> bool {
        self.deflate(&mut v);
        let mut mv = spmv(m, &v);
        let nrm = dot(&v, &mv).max(0.0).sqrt();
        if nrm < 0.5 {
            // Already spanned by locked eigenvectors.
            return false;
        }
        let inv = 1.0 / nrm;
        for x in &mut v {
            *x *= inv;
        }
        for x in &mut mv {
            *x *= inv;
        }
        self.evals.push(lam);
        self.vecs.push(v);
        self.m_vecs.push(mv);
        true
    }
}

/// Deterministic xorshift64* PRNG for reproducible starting vectors.
fn random_start_vec(n: usize, seed: u64) -> Vec<f64> {
    let mut state = seed ^ 0x9e37_79b9_7f4a_7c15;
    if state == 0 {
        state = 0xdead_beef_cafe_babe;
    }
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let bits = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
        let u = (bits >> 11) as f64 * (1.0 / (1u64 << 53) as f64);
        v.push(2.0 * u - 1.0);
    }
    v
}

/// Run one deflated shift-invert Lanczos pass at `sigma` using `factor`, locking any
/// newly converged eigenpairs in `[win_lo, win_hi]` (and any other converged pairs near `sigma`).
/// Returns the number of newly locked eigenpairs that fall in `[win_lo, win_hi]`.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::needless_range_loop)]
fn lanczos_pass(
    k: &SparseMatrix,
    m: &SparseMatrix,
    sigma: f64,
    factor: &ShiftedFactor,
    win_lo: f64,
    win_hi: f64,
    global_lo: f64,
    global_hi: f64,
    want: usize,
    seed: u64,
    locked: &mut LockedSpace,
    e: &Engine,
) -> Result<usize> {
    let n = k.rows();
    let avail = n.saturating_sub(locked.len());
    if avail == 0 {
        return Ok(0);
    }
    let m_steps = avail.min((2 * want + 28).clamp(40, 120));

    // Build M-normalized, deflated starting vector q_0.
    let mut q0 = random_start_vec(n, seed);
    locked.deflate(&mut q0);
    let mut p0 = spmv(m, &q0);
    let nrm0 = dot(&q0, &p0).max(0.0).sqrt();
    if !(nrm0.is_finite() && nrm0 > 1e-14) {
        return Ok(0);
    }
    for x in &mut q0 {
        *x /= nrm0;
    }
    for x in &mut p0 {
        *x /= nrm0;
    }

    let mut q_basis: Vec<Vec<f64>> = Vec::with_capacity(m_steps);
    let mut p_basis: Vec<Vec<f64>> = Vec::with_capacity(m_steps);
    let mut alphas: Vec<f64> = Vec::with_capacity(m_steps);
    let mut betas: Vec<f64> = Vec::with_capacity(m_steps);

    q_basis.push(q0);
    p_basis.push(p0);

    let mut last_beta = 0.0;
    for j in 0..m_steps {
        e.check_cancel()?;
        let mut z = factor.solve(&p_basis[j])?;
        // Deflate against all locked eigenvectors.
        locked.deflate(&mut z);

        let alpha = dot(&z, &p_basis[j]);
        alphas.push(alpha);

        // Two-pass CGS full reorthogonalization in M-inner product against q_0..=q_j.
        for _ in 0..2 {
            for i in 0..=j {
                let h = dot(&z, &p_basis[i]);
                if h != 0.0 {
                    for (zi, &qi) in z.iter_mut().zip(q_basis[i].iter()) {
                        *zi -= h * qi;
                    }
                }
            }
        }
        // Keep deflated after reorthogonalization.
        locked.deflate(&mut z);

        let mut p_next = spmv(m, &z);
        let beta = dot(&z, &p_next).max(0.0).sqrt();
        last_beta = beta;

        if j + 1 == m_steps || beta <= 1e-13 * alpha.abs().max(1.0) {
            break;
        }
        betas.push(beta);
        let inv_b = 1.0 / beta;
        for x in &mut z {
            *x *= inv_b;
        }
        for x in &mut p_next {
            *x *= inv_b;
        }
        q_basis.push(z);
        p_basis.push(p_next);
    }

    let dim = alphas.len();
    if dim == 0 {
        return Ok(0);
    }

    // Solve the small symmetric tridiagonal Ritz eigenproblem T_dim Y = Y Theta.
    let mut t_mat = DMatrix::zeros(dim, dim);
    for i in 0..dim {
        t_mat[(i, i)] = alphas[i];
        if i + 1 < dim {
            t_mat[(i, i + 1)] = betas[i];
            t_mat[(i + 1, i)] = betas[i];
        }
    }
    let eig = SymmetricEigen::new(t_mat);

    // Order Ritz values by proximity to sigma (largest |theta| first).
    let mut order: Vec<usize> = (0..dim).collect();
    order.sort_by(|&a, &b| {
        eig.eigenvalues[b]
            .abs()
            .total_cmp(&eig.eigenvalues[a].abs())
    });

    // ||K||_1 / ||M||_1 estimates the eigenvalue scale. The residual denominator
    // uses these operator norms times ||v||, not ||Kv||: ||Kv|| vanishes with λ,
    // so a pure-Neumann constant mode (λ = 0) never passed EIG_RES_TOL.
    let k_norm = norm1(k);
    let m_norm = norm1(m);

    let mut added_in_win = 0usize;
    for idx in order {
        let theta = eig.eigenvalues[idx];
        if !theta.is_finite() || theta.abs() < 1e-14 {
            continue;
        }
        let lam_est = sigma + 1.0 / theta;
        // Skip Ritz values far outside the global search range.
        let margin = 0.05 * (global_hi - global_lo).abs().clamp(1.0, 1e6);
        if lam_est < global_lo - margin || lam_est > global_hi + margin {
            continue;
        }
        // Quick Ritz residual filter on T_sigma before reconstructing the full vector.
        let ritz_res = (last_beta * eig.eigenvectors[(dim - 1, idx)]).abs();
        if ritz_res > 1e-4 * theta.abs() {
            continue;
        }

        // Reconstruct Ritz vector v = Q * y.
        let mut v = vec![0.0; n];
        for r in 0..dim {
            let yr = eig.eigenvectors[(r, idx)];
            if yr == 0.0 {
                continue;
            }
            for (vi, &qri) in v.iter_mut().zip(q_basis[r].iter()) {
                *vi += yr * qri;
            }
        }
        let mv = spmv(m, &v);
        let norm_m = dot(&v, &mv).max(0.0).sqrt();
        if !(norm_m.is_finite() && norm_m > 0.0) {
            continue;
        }
        for x in &mut v {
            *x /= norm_m;
        }
        let mv: Vec<f64> = mv.into_iter().map(|x| x / norm_m).collect();
        let kv = spmv(k, &v);
        let lam = dot(&v, &kv);

        // Scale-aware generalized residual
        // ||Kv - λ Mv|| / ((||K||_1 + |λ| ||M||_1) ||v||).
        let mut res_sq = 0.0;
        for (&kvi, &mvi) in kv.iter().zip(mv.iter()) {
            let d = kvi - lam * mvi;
            res_sq += d * d;
        }
        let denom = (k_norm + lam.abs() * m_norm) * norm2(&v);
        let rel_res = res_sq.sqrt() / denom.max(1e-30);
        if rel_res <= EIG_RES_TOL && locked.try_add(lam, v, m) {
            if lam >= win_lo && lam <= win_hi {
                added_in_win += 1;
            }
            if locked.len() >= MAX_EIGENPAIRS {
                return Err(format!(
                    "solvepdeeig: number of eigenvalues in range exceeds limit ({MAX_EIGENPAIRS})"
                ));
            }
        }
    }

    Ok(added_in_win)
}

fn solve_sparse_shift_invert(
    k: &SparseMatrix,
    m: &SparseMatrix,
    lo: f64,
    hi: f64,
    e: &mut Engine,
) -> Result<Vec<(f64, Vec<f64>)>> {
    // Verify M is SPD up front.
    verify_sparse_spd(m, e)?;

    let pad_hi = 1e-7 * hi.abs().max(1.0);
    let hi_plus = hi + pad_hi;
    let (hi_shift, hi_fac) = factorize_shifted_retry(k, m, hi_plus, e)?;
    let count_hi = hi_fac.inertia();

    let (lo_shift, count_lo) = if lo.is_finite() {
        let pad_lo = 1e-7 * lo.abs().max(1.0);
        let lo_minus = lo - pad_lo;
        let (ls, lf) = factorize_shifted_retry(k, m, lo_minus, e)?;
        (ls, lf.inertia())
    } else {
        (f64::NEG_INFINITY, Some(0))
    };

    // Fast path: Sylvester inertia proves the range is empty!
    if let (Some(c_lo), Some(c_hi)) = (count_lo, count_hi) {
        if c_hi <= c_lo {
            return Ok(Vec::new());
        }
        if c_hi - c_lo > MAX_EIGENPAIRS {
            return Err(format!(
                "solvepdeeig: {} eigenvalues in range exceeds limit ({MAX_EIGENPAIRS})",
                c_hi - c_lo
            ));
        }
    }

    let mut locked = LockedSpace::new();
    let mut shifts_used = 0usize;
    let mut seed_counter = 1u64;

    if let (Some(c_lo), Some(c_hi)) = (count_lo, count_hi) {
        // Spectrum-sliced shift-invert Lanczos guided by exact Sylvester inertia.
        // Queue entries: (a, b, inertia_a, inertia_b).
        let mut queue: VecDeque<(f64, f64, usize, usize)> = VecDeque::new();
        queue.push_back((lo_shift, hi_shift, c_lo, c_hi));

        while let Some((a, b, ca, cb)) = queue.pop_front() {
            e.check_cancel()?;
            if cb <= ca {
                continue;
            }
            let need = cb - ca;
            if locked.count_in(a, b) >= need {
                continue;
            }
            if shifts_used >= MAX_SHIFTS {
                return Err(format!(
                    "solvepdeeig: failed to converge all eigenvalues within {MAX_SHIFTS} shifts"
                ));
            }
            shifts_used += 1;

            // Choose shift at midpoint of [a, b], or sensible anchor when a = -Inf.
            let sigma_target = if a.is_finite() {
                0.5 * (a + b)
            } else if b > 0.0 {
                0.5 * b
            } else {
                b - b.abs().max(1.0)
            };

            let (sigma, factor) = factorize_shifted_retry(k, m, sigma_target, e)?;

            // Run up to 3 deflated Lanczos passes on the same factorization (handles multiplicities).
            let global_lo = if lo.is_finite() {
                lo
            } else {
                sigma.min(0.0) - 1e6
            };
            for _ in 0..3 {
                let remaining = need.saturating_sub(locked.count_in(a, b));
                if remaining == 0 {
                    break;
                }
                seed_counter += 1;
                let added = lanczos_pass(
                    k,
                    m,
                    sigma,
                    &factor,
                    a,
                    b,
                    global_lo,
                    hi_shift,
                    remaining,
                    seed_counter,
                    &mut locked,
                    e,
                )?;
                if added == 0 {
                    break;
                }
            }

            if locked.count_in(a, b) >= need {
                continue;
            }

            // Subdivide using exact inertia at sigma.
            if let Some(c_mid) = factor.inertia() {
                if c_mid > ca && c_mid < cb {
                    queue.push_back((a, sigma, ca, c_mid));
                    queue.push_back((sigma, b, c_mid, cb));
                } else if c_mid == ca && sigma > a {
                    queue.push_back((sigma, b, c_mid, cb));
                } else if c_mid == cb && sigma < b {
                    if a.is_finite() {
                        queue.push_back((a, sigma, ca, c_mid));
                    } else {
                        // All eigenvalues are strictly below sigma; step further left.
                        queue.push_back((f64::NEG_INFINITY, sigma, ca, c_mid));
                    }
                } else {
                    // Edge case: tiny interval or all remaining clustered right at sigma.
                    let remaining = need.saturating_sub(locked.count_in(a, b));
                    if remaining > 0 {
                        seed_counter += 1;
                        let _ = lanczos_pass(
                            k,
                            m,
                            sigma,
                            &factor,
                            a,
                            b,
                            global_lo,
                            hi_shift,
                            remaining + 8,
                            seed_counter,
                            &mut locked,
                            e,
                        )?;
                    }
                }
            } else {
                // No inertia (sparse LU fallback); split geometrically if finite.
                if a.is_finite() && (b - a) > 1e-9 * b.abs().max(1.0) {
                    let mid = 0.5 * (a + b);
                    queue.push_back((a, mid, ca, cb));
                    queue.push_back((mid, b, ca, cb));
                }
            }
        }
    } else {
        // Fallback when boundary inertia was unavailable (LU only): shift at midpoint
        // and collect until hi is passed.
        let sigma0 = if lo.is_finite() {
            0.5 * (lo + hi)
        } else if hi > 0.0 {
            0.5 * hi
        } else {
            hi - 1.0
        };
        let (sigma, factor) = factorize_shifted_retry(k, m, sigma0, e)?;
        let global_lo = if lo.is_finite() {
            lo
        } else {
            sigma.min(0.0) - 1e6
        };
        for _ in 0..4 {
            seed_counter += 1;
            let added = lanczos_pass(
                k,
                m,
                sigma,
                &factor,
                lo,
                hi,
                global_lo,
                hi_shift,
                24,
                seed_counter,
                &mut locked,
                e,
            )?;
            if added == 0 {
                break;
            }
        }
    }

    // Filter strictly to [lo, hi] and sort ascending.
    let mut pairs: Vec<(f64, Vec<f64>)> = locked
        .evals
        .into_iter()
        .zip(locked.vecs)
        .filter(|(lam, _)| *lam >= lo && *lam <= hi)
        .collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

    // Sylvester count c(σ) = #{λ : λ < σ}. When both endpoint factorizations
    // supplied inertia, c_hi - c_lo is the number of eigenvalues in
    // [lo_shift, hi_shift). That window contains [lo, hi] (the shift pad is
    // ~1e-7 relative). A short locked set means an unresolved slice was
    // dropped; never return that partial set as success. The caller uses the
    // dense fallback for n <= DENSE_FALLBACK_MAX and otherwise propagates
    // this error.
    match (count_lo, count_hi) {
        (Some(c_lo), Some(c_hi)) if pairs.len() == c_hi.saturating_sub(c_lo) => Ok(pairs),
        _ => Err("solvepdeeig: failed to converge all eigenvalues in range".into()),
    }
}
