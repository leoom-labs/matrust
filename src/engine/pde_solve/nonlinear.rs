//! Nonlinear stationary 2-D PDE solver (damped Newton with exact quadrature-point
//! chain-rule sparse Jacobian and Armijo backtracking line search).
use super::assemble::{
    BoundaryFacet, DomainQuadPoint, EdgeQuadPoint, apply_dirichlet_elimination, assemble_kmf_sys,
    assemble_neumann_sys, solve_sparse_system, sparse_matvec_add,
};
use super::quadrature::{
    TRI_QUAD_P1, TRI_QUAD_P2, p1_jacobian_and_grads, p1_shape, p2_jacobian_and_grads, p2_shape,
};
use super::types::{CoeffAssignment, CoeffC, CoeffVal, SolverOptions};
use super::{
    EdgeNeumannEval, check_coeff_workspace, eval_c_sys_with_state, eval_mat_sys_with_state,
    eval_vec_sys_with_state,
};
use crate::Result;
use crate::engine::{Engine, Execution, pde_mesh::FeMesh};
use crate::sparse::SparseMatrix;
use std::sync::atomic::{AtomicBool, Ordering};

/// State `(u, ux, uy)` at a collection of evaluation points.
/// Stored in component-major-per-point layout: for point `q` and component `c`,
/// value is at `q * n_comp + c` (matching MATLAB's `N x nr` column-major layout!).
pub(crate) struct QuadState {
    pub(crate) u: Vec<f64>,
    pub(crate) ux: Vec<f64>,
    pub(crate) uy: Vec<f64>,
    pub(crate) n_comp: usize,
    pub(crate) n_pts: usize,
}

impl QuadState {
    pub(crate) fn zeros(n_comp: usize, n_pts: usize) -> Self {
        Self {
            u: vec![0.0; n_comp * n_pts],
            ux: vec![0.0; n_comp * n_pts],
            uy: vec![0.0; n_comp * n_pts],
            n_comp,
            n_pts,
        }
    }

    pub(crate) fn slice_indices(&self, indices: &[usize]) -> Self {
        let nc = self.n_comp;
        let n = indices.len();
        let mut u = Vec::with_capacity(nc * n);
        let mut ux = Vec::with_capacity(nc * n);
        let mut uy = Vec::with_capacity(nc * n);
        for &idx in indices {
            let b = idx * nc;
            u.extend_from_slice(&self.u[b..b + nc]);
            ux.extend_from_slice(&self.ux[b..b + nc]);
            uy.extend_from_slice(&self.uy[b..b + nc]);
        }
        Self {
            u,
            ux,
            uy,
            n_comp: nc,
            n_pts: n,
        }
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Execution cancelled".into())
    } else {
        Ok(())
    }
}

/// Evaluate `(u, ux, uy)` at every domain quadrature point from full nodal vector `u_nodal`
/// (component-major: `comp * n_nodes + node`).
#[allow(clippy::needless_range_loop)]
pub(crate) fn eval_state_at_domain_quad(
    mesh: &FeMesh,
    n_comp: usize,
    u_nodal: &[f64],
    cancel: &AtomicBool,
) -> Result<QuadState> {
    let n_nodes = mesh.nodes.len();
    let n_elems = mesh.tris.len();
    if mesh.quadratic {
        let mids = mesh
            .mid
            .as_ref()
            .ok_or("solvepde: quadratic mesh is missing midside nodes")?;
        let nq = TRI_QUAD_P2.len();
        let n_pts = n_elems * nq;
        let shapes: Vec<([f64; 6], [f64; 6], [f64; 6])> = TRI_QUAD_P2
            .iter()
            .map(|qp| p2_shape(qp.xi, qp.eta))
            .collect();
        let mut state = QuadState::zeros(n_comp, n_pts);
        for t in 0..n_elems {
            if t % 1024 == 0 {
                check_cancel(cancel)?;
            }
            let c = mesh.tris[t];
            let m = mids[t];
            let dofs = [c[0], c[1], c[2], m[0], m[1], m[2]];
            let coords = [
                mesh.nodes[dofs[0]],
                mesh.nodes[dofs[1]],
                mesh.nodes[dofs[2]],
                mesh.nodes[dofs[3]],
                mesh.nodes[dofs[4]],
                mesh.nodes[dofs[5]],
            ];
            for q in 0..nq {
                let idx = t * nq + q;
                let (n, dxi, deta) = &shapes[q];
                let (_, dnx, dny) = p2_jacobian_and_grads(&coords, dxi, deta)?;
                let base = idx * n_comp;
                for ci in 0..n_comp {
                    let off = ci * n_nodes;
                    let mut uv = 0.0;
                    let mut uxv = 0.0;
                    let mut uyv = 0.0;
                    for a in 0..6 {
                        let ua = u_nodal[off + dofs[a]];
                        uv += n[a] * ua;
                        uxv += dnx[a] * ua;
                        uyv += dny[a] * ua;
                    }
                    state.u[base + ci] = uv;
                    state.ux[base + ci] = uxv;
                    state.uy[base + ci] = uyv;
                }
            }
        }
        Ok(state)
    } else {
        let nq = TRI_QUAD_P1.len();
        let n_pts = n_elems * nq;
        let shapes: Vec<[f64; 3]> = TRI_QUAD_P1
            .iter()
            .map(|qp| p1_shape(qp.xi, qp.eta))
            .collect();
        let mut state = QuadState::zeros(n_comp, n_pts);
        for t in 0..n_elems {
            if t % 2048 == 0 {
                check_cancel(cancel)?;
            }
            let dofs = mesh.tris[t];
            let coords = [
                mesh.nodes[dofs[0]],
                mesh.nodes[dofs[1]],
                mesh.nodes[dofs[2]],
            ];
            let (_, dnx, dny) = p1_jacobian_and_grads(&coords)?;
            for q in 0..nq {
                let idx = t * nq + q;
                let n = &shapes[q];
                let base = idx * n_comp;
                for ci in 0..n_comp {
                    let off = ci * n_nodes;
                    let u0 = u_nodal[off + dofs[0]];
                    let u1 = u_nodal[off + dofs[1]];
                    let u2 = u_nodal[off + dofs[2]];
                    state.u[base + ci] = n[0] * u0 + n[1] * u1 + n[2] * u2;
                    state.ux[base + ci] = dnx[0] * u0 + dnx[1] * u1 + dnx[2] * u2;
                    state.uy[base + ci] = dny[0] * u0 + dny[1] * u1 + dny[2] * u2;
                }
            }
        }
        Ok(state)
    }
}

/// Evaluate domain `(c, a, f)` at all domain quadrature points given `state`.
#[allow(clippy::too_many_arguments)]
fn eval_domain_coeffs(
    e: &mut Engine,
    out: &mut Execution,
    n_comp: usize,
    coeff_assignments: &[CoeffAssignment],
    dom_pts: &[DomainQuadPoint],
    group_indices: &[Vec<usize>],
    state: &QuadState,
) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    let n_dom = dom_pts.len();
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
        let sub_state = state.slice_indices(indices);

        let cv = eval_c_sys_with_state(
            e,
            out,
            &ca.c,
            n_comp,
            &xs,
            &ys,
            &subs,
            f64::NAN,
            Some(&sub_state),
        )?;
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
            Some(&sub_state),
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
            Some(&sub_state),
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
    Ok((c_vals, a_vals, f_vals))
}

/// Assemble the exact tangent Jacobian `J = dR/dU` of the domain terms using
/// quadrature-point finite differences of any handle-valued `(c, a, f)` w.r.t. `(u, ux, uy)`.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::needless_range_loop)]
fn assemble_domain_jacobian(
    e: &mut Engine,
    out: &mut Execution,
    mesh: &FeMesh,
    n_comp: usize,
    coeff_assignments: &[CoeffAssignment],
    dom_pts: &[DomainQuadPoint],
    group_indices: &[Vec<usize>],
    state: &QuadState,
    c_vals: &[f64],
    a_vals: &[f64],
    f_vals: &[f64],
    base_k_plus_m: &SparseMatrix,
) -> Result<SparseMatrix> {
    let any_c_handle = coeff_assignments
        .iter()
        .any(|ca| matches!(ca.c, CoeffC::Handle(_)));
    let any_a_handle = coeff_assignments
        .iter()
        .any(|ca| matches!(ca.a, CoeffVal::Handle(_)));
    let any_f_handle = coeff_assignments
        .iter()
        .any(|ca| matches!(ca.f, CoeffVal::Handle(_)));
    if !any_c_handle && !any_a_handle && !any_f_handle {
        return Ok(base_k_plus_m.clone());
    }

    let n_dom = dom_pts.len();
    let n_nodes = mesh.nodes.len();
    let n_dofs = n_nodes * n_comp;
    let m2 = 2 * n_comp;
    let c_stride = m2 * m2;
    let a_stride = n_comp * n_comp;

    // For each component cj and each derivative kind (0: u, 1: ux, 2: uy),
    // compute dc/ds, da/ds, df/ds at all quadrature points.
    let n_vars = 3 * n_comp;
    let mut dc_list: Vec<Option<Vec<f64>>> = vec![None; n_vars];
    let mut da_list: Vec<Option<Vec<f64>>> = vec![None; n_vars];
    let mut df_list: Vec<Option<Vec<f64>>> = vec![None; n_vars];

    for kind in 0..3 {
        for cj in 0..n_comp {
            let var_idx = kind * n_comp + cj;
            let mut pert = QuadState {
                u: state.u.clone(),
                ux: state.ux.clone(),
                uy: state.uy.clone(),
                n_comp,
                n_pts: state.n_pts,
            };
            let mut steps = vec![0.0; n_dom];
            let target = match kind {
                0 => &mut pert.u,
                1 => &mut pert.ux,
                _ => &mut pert.uy,
            };
            for q in 0..n_dom {
                let v = target[q * n_comp + cj];
                let h = 1e-7 * v.abs().max(1.0);
                target[q * n_comp + cj] = v + h;
                steps[q] = h;
            }
            let (cp, ap, fp) = eval_domain_coeffs(
                e,
                out,
                n_comp,
                coeff_assignments,
                dom_pts,
                group_indices,
                &pert,
            )?;

            if any_c_handle {
                let mut dc = vec![0.0; n_dom * c_stride];
                let mut nz = false;
                for q in 0..n_dom {
                    let inv_h = 1.0 / steps[q];
                    let b = q * c_stride;
                    for k in 0..c_stride {
                        let d = (cp[b + k] - c_vals[b + k]) * inv_h;
                        if d != 0.0 {
                            dc[b + k] = d;
                            nz = true;
                        }
                    }
                }
                if nz {
                    dc_list[var_idx] = Some(dc);
                }
            }
            if any_a_handle {
                let mut da = vec![0.0; n_dom * a_stride];
                let mut nz = false;
                for q in 0..n_dom {
                    let inv_h = 1.0 / steps[q];
                    let b = q * a_stride;
                    for k in 0..a_stride {
                        let d = (ap[b + k] - a_vals[b + k]) * inv_h;
                        if d != 0.0 {
                            da[b + k] = d;
                            nz = true;
                        }
                    }
                }
                if nz {
                    da_list[var_idx] = Some(da);
                }
            }
            if any_f_handle {
                let mut df = vec![0.0; n_dom * n_comp];
                let mut nz = false;
                for q in 0..n_dom {
                    let inv_h = 1.0 / steps[q];
                    let b = q * n_comp;
                    for k in 0..n_comp {
                        let d = (fp[b + k] - f_vals[b + k]) * inv_h;
                        if d != 0.0 {
                            df[b + k] = d;
                            nz = true;
                        }
                    }
                }
                if nz {
                    df_list[var_idx] = Some(df);
                }
            }
        }
    }

    let any_deriv = dc_list.iter().any(Option::is_some)
        || da_list.iter().any(Option::is_some)
        || df_list.iter().any(Option::is_some);
    if !any_deriv {
        return Ok(base_k_plus_m.clone());
    }

    // Assemble the nonlinear tangent correction matrix dJ and add to base_k_plus_m.
    let n_elems = mesh.tris.len();
    let mut triplets: Vec<(usize, usize, f64)> = Vec::new();
    let cancel = &e.cancel;

    // Helper closure for one quadrature point contribution to local tangent matrix je.
    let accum_qp =
        |idx: usize, w: f64, n_loc: usize, n: &[f64], dnx: &[f64], dny: &[f64], je: &mut [f64]| {
            let nd_loc = n_loc * n_comp;
            let sb = idx * n_comp;
            let u_q = &state.u[sb..sb + n_comp];
            let ux_q = &state.ux[sb..sb + n_comp];
            let uy_q = &state.uy[sb..sb + n_comp];

            for kind in 0..3 {
                for cj in 0..n_comp {
                    let var_idx = kind * n_comp + cj;
                    let dc_opt = dc_list[var_idx].as_deref();
                    let da_opt = da_list[var_idx].as_deref();
                    let df_opt = df_list[var_idx].as_deref();
                    if dc_opt.is_none() && da_opt.is_none() && df_opt.is_none() {
                        continue;
                    }
                    let basis_j = match kind {
                        0 => n,
                        1 => dnx,
                        _ => dny,
                    };
                    for ci in 0..n_comp {
                        let mut dfx = 0.0;
                        let mut dfy = 0.0;
                        if let Some(dc) = dc_opt {
                            let cb = idx * c_stride;
                            let r0 = 2 * ci;
                            for ck in 0..n_comp {
                                let c0 = 2 * ck;
                                let c11 = dc[cb + c0 * m2 + r0];
                                let c21 = dc[cb + c0 * m2 + r0 + 1];
                                let c12 = dc[cb + (c0 + 1) * m2 + r0];
                                let c22 = dc[cb + (c0 + 1) * m2 + r0 + 1];
                                dfx += c11 * ux_q[ck] + c12 * uy_q[ck];
                                dfy += c21 * ux_q[ck] + c22 * uy_q[ck];
                            }
                        }
                        let mut dreac = 0.0;
                        if let Some(da) = da_opt {
                            let ab = idx * a_stride;
                            for ck in 0..n_comp {
                                dreac += da[ab + ck * n_comp + ci] * u_q[ck];
                            }
                        }
                        if let Some(df) = df_opt {
                            dreac -= df[idx * n_comp + ci];
                        }
                        if dfx == 0.0 && dfy == 0.0 && dreac == 0.0 {
                            continue;
                        }
                        for i in 0..n_loc {
                            let row = ci * n_loc + i;
                            let test = w * (dnx[i] * dfx + dny[i] * dfy + n[i] * dreac);
                            if test != 0.0 {
                                for j in 0..n_loc {
                                    let col = cj * n_loc + j;
                                    je[row * nd_loc + col] += test * basis_j[j];
                                }
                            }
                        }
                    }
                }
            }
        };

    if mesh.quadratic {
        let mids = mesh
            .mid
            .as_ref()
            .ok_or("solvepde: quadratic mesh is missing midside nodes")?;
        let nq = TRI_QUAD_P2.len();
        let shapes: Vec<([f64; 6], [f64; 6], [f64; 6])> = TRI_QUAD_P2
            .iter()
            .map(|qp| p2_shape(qp.xi, qp.eta))
            .collect();
        let nd_loc = 6 * n_comp;
        for t in 0..n_elems {
            if t % 512 == 0 {
                check_cancel(cancel)?;
            }
            let c = mesh.tris[t];
            let m = mids[t];
            let dofs = [c[0], c[1], c[2], m[0], m[1], m[2]];
            let coords = [
                mesh.nodes[dofs[0]],
                mesh.nodes[dofs[1]],
                mesh.nodes[dofs[2]],
                mesh.nodes[dofs[3]],
                mesh.nodes[dofs[4]],
                mesh.nodes[dofs[5]],
            ];
            let mut je = vec![0.0; nd_loc * nd_loc];
            for (q, qp) in TRI_QUAD_P2.iter().enumerate() {
                let (n, dxi, deta) = &shapes[q];
                let (det, dnx, dny) = p2_jacobian_and_grads(&coords, dxi, deta)?;
                accum_qp(t * nq + q, qp.w * det, 6, n, &dnx, &dny, &mut je);
            }
            for ci in 0..n_comp {
                for i in 0..6 {
                    let row = ci * 6 + i;
                    let gi = ci * n_nodes + dofs[i];
                    for cj in 0..n_comp {
                        for j in 0..6 {
                            let v = je[row * nd_loc + cj * 6 + j];
                            if v != 0.0 {
                                triplets.push((gi, cj * n_nodes + dofs[j], v));
                            }
                        }
                    }
                }
            }
        }
    } else {
        let nq = TRI_QUAD_P1.len();
        let shapes: Vec<[f64; 3]> = TRI_QUAD_P1
            .iter()
            .map(|qp| p1_shape(qp.xi, qp.eta))
            .collect();
        let nd_loc = 3 * n_comp;
        for t in 0..n_elems {
            if t % 1024 == 0 {
                check_cancel(cancel)?;
            }
            let dofs = mesh.tris[t];
            let coords = [
                mesh.nodes[dofs[0]],
                mesh.nodes[dofs[1]],
                mesh.nodes[dofs[2]],
            ];
            let (det, dnx, dny) = p1_jacobian_and_grads(&coords)?;
            let mut je = vec![0.0; nd_loc * nd_loc];
            for (q, qp) in TRI_QUAD_P1.iter().enumerate() {
                accum_qp(t * nq + q, qp.w * det, 3, &shapes[q], &dnx, &dny, &mut je);
            }
            for ci in 0..n_comp {
                for i in 0..3 {
                    let row = ci * 3 + i;
                    let gi = ci * n_nodes + dofs[i];
                    for cj in 0..n_comp {
                        for j in 0..3 {
                            let v = je[row * nd_loc + cj * 3 + j];
                            if v != 0.0 {
                                triplets.push((gi, cj * n_nodes + dofs[j], v));
                            }
                        }
                    }
                }
            }
        }
    }

    let dj = SparseMatrix::from_triplets(n_dofs, n_dofs, &triplets, false, cancel)?;
    base_k_plus_m.add(&dj, 1.0)
}

/// Solve a nonlinear stationary 2-D PDE system using damped Newton iteration.
#[allow(clippy::too_many_arguments)]
pub(crate) fn solve_nonlinear_stationary(
    e: &mut Engine,
    out: &mut Execution,
    mesh: &FeMesh,
    n_comp: usize,
    coeff_assignments: &[CoeffAssignment],
    dom_pts: &[DomainQuadPoint],
    group_indices: &[Vec<usize>],
    neumann_facets: &[BoundaryFacet],
    edge_pts: &[EdgeQuadPoint],
    neumann_eval: &EdgeNeumannEval,
    dirichlet_mask: &[bool],
    dirichlet_vals: &[f64],
    opts: SolverOptions,
) -> Result<Vec<f64>> {
    let n_nodes = mesh.nodes.len();
    let n_dofs = n_nodes * n_comp;

    // Initial iterate: 0 on free DOFs, exact Dirichlet values on constrained DOFs.
    let mut u = vec![0.0; n_dofs];
    for i in 0..n_dofs {
        if dirichlet_mask[i] {
            u[i] = dirichlet_vals[i];
        }
    }

    // Free DOF list for computing the reduced residual norm.
    let free_dofs: Vec<usize> = (0..n_dofs).filter(|&i| !dirichlet_mask[i]).collect();
    if free_dofs.is_empty() {
        return Ok(u);
    }

    // Assemble state-independent Neumann terms once if present.
    let (const_q_mat, const_g_vec) = if !neumann_facets.is_empty() && !neumann_eval.is_state_dep {
        let (qm, gv) = assemble_neumann_sys(
            mesh,
            n_comp,
            neumann_facets,
            &neumann_eval.const_q,
            &neumann_eval.const_g,
            &e.cancel,
        )?;
        (Some(qm), Some(gv))
    } else {
        (None, None)
    };

    // Helper to evaluate residual R(u) = A(u)*u - b(u) on all DOFs and its free-DOF L2 norm.
    #[allow(clippy::type_complexity)]
    let eval_system = |e: &mut Engine,
                       out: &mut Execution,
                       u_cur: &[f64]|
     -> Result<(
        QuadState,
        Vec<f64>,
        Vec<f64>,
        Vec<f64>,
        SparseMatrix,
        Vec<f64>,
        Vec<f64>,
        f64,
    )> {
        let state = eval_state_at_domain_quad(mesh, n_comp, u_cur, &e.cancel)?;
        let (c_vals, a_vals, f_vals) = eval_domain_coeffs(
            e,
            out,
            n_comp,
            coeff_assignments,
            dom_pts,
            group_indices,
            &state,
        )?;
        let sys = assemble_kmf_sys(mesh, n_comp, &c_vals, &a_vals, &f_vals, &e.cancel)?;
        let mut total_a = sys.k.add(&sys.m, 1.0)?;
        let mut total_b = sys.f;
        if let (Some(qm), Some(gv)) = (&const_q_mat, &const_g_vec) {
            if qm.nnz() > 0 {
                total_a = total_a.add(qm, 1.0)?;
            }
            for (bi, &gi) in total_b.iter_mut().zip(gv.iter()) {
                *bi += gi;
            }
        } else if !neumann_facets.is_empty() {
            let (qv, gv_pts) = neumann_eval.eval_at_u(e, out, mesh, edge_pts, u_cur)?;
            let (qm, gv) =
                assemble_neumann_sys(mesh, n_comp, neumann_facets, &qv, &gv_pts, &e.cancel)?;
            if qm.nnz() > 0 {
                total_a = total_a.add(&qm, 1.0)?;
            }
            for (bi, &gi) in total_b.iter_mut().zip(gv.iter()) {
                *bi += gi;
            }
        }
        // R = A * u - b
        let mut res = vec![0.0; n_dofs];
        sparse_matvec_add(&total_a, u_cur, 1.0, &mut res);
        for i in 0..n_dofs {
            res[i] -= total_b[i];
        }
        let mut norm2 = 0.0;
        for &fi in &free_dofs {
            let r = res[fi];
            norm2 += r * r;
        }
        let res_norm = norm2.sqrt();
        Ok((
            state, c_vals, a_vals, f_vals, total_a, total_b, res, res_norm,
        ))
    };

    let (
        mut state,
        mut c_vals,
        mut a_vals,
        mut f_vals,
        mut total_a,
        mut total_b,
        mut res,
        mut res_norm,
    ) = eval_system(e, out, &u)?;

    if res_norm <= opts.residual_tolerance {
        return Ok(u);
    }

    let zero_dir = vec![0.0; n_dofs];

    for _iter in 0..opts.max_iterations {
        e.check_cancel()?;

        // Assemble exact tangent Jacobian J = dR/dU.
        let jac_full = assemble_domain_jacobian(
            e,
            out,
            mesh,
            n_comp,
            coeff_assignments,
            dom_pts,
            group_indices,
            &state,
            &c_vals,
            &a_vals,
            &f_vals,
            &total_a,
        )?;

        // Solve J_II * du_I = -R_I (with homogeneous Dirichlet on du).
        let neg_res: Vec<f64> = res.iter().map(|&r| -r).collect();
        let reduced =
            apply_dirichlet_elimination(&jac_full, &neg_res, dirichlet_mask, &zero_dir, &e.cancel)?;
        let du_free = match solve_sparse_system(e, &reduced.a_ii, &reduced.b_eff, out) {
            Ok(v) => v,
            Err(_) => {
                // Fallback: Picard step A(u) * u_new = b(u) => du = u_new - u.
                let red_p = apply_dirichlet_elimination(
                    &total_a,
                    &total_b,
                    dirichlet_mask,
                    dirichlet_vals,
                    &e.cancel,
                )?;
                let u_pic = solve_sparse_system(e, &red_p.a_ii, &red_p.b_eff, out)?;
                red_p
                    .free_dofs
                    .iter()
                    .zip(u_pic.iter())
                    .map(|(&dof, &val)| val - u[dof])
                    .collect()
            }
        };

        // Armijo backtracking line search.
        let mut alpha = 1.0;
        let mut accepted = false;
        let mut u_trial = u.clone();

        while alpha >= opts.min_step {
            u_trial.copy_from_slice(&u);
            for (&dof, &dval) in reduced.free_dofs.iter().zip(du_free.iter()) {
                u_trial[dof] = u[dof] + alpha * dval;
            }
            if let Ok((st_t, cv_t, av_t, fv_t, ta_t, tb_t, res_t, rn_t)) =
                eval_system(e, out, &u_trial)
                && rn_t.is_finite()
                && (rn_t < res_norm || rn_t <= opts.residual_tolerance)
            {
                u = u_trial;
                state = st_t;
                c_vals = cv_t;
                a_vals = av_t;
                f_vals = fv_t;
                total_a = ta_t;
                total_b = tb_t;
                res = res_t;
                res_norm = rn_t;
                accepted = true;
                break;
            }
            alpha *= 0.5;
        }

        if !accepted {
            // Try one damped Picard step before giving up.
            let red_p = apply_dirichlet_elimination(
                &total_a,
                &total_b,
                dirichlet_mask,
                dirichlet_vals,
                &e.cancel,
            )?;
            let u_pic = solve_sparse_system(e, &red_p.a_ii, &red_p.b_eff, out)?;
            let mut u_try = u.clone();
            for (&dof, &val) in red_p.free_dofs.iter().zip(u_pic.iter()) {
                u_try[dof] = 0.5 * (u[dof] + val);
            }
            let (st_t, cv_t, av_t, fv_t, ta_t, tb_t, res_t, rn_t) = eval_system(e, out, &u_try)?;
            if rn_t.is_finite() && rn_t < res_norm {
                u = u_try;
                state = st_t;
                c_vals = cv_t;
                a_vals = av_t;
                f_vals = fv_t;
                total_a = ta_t;
                total_b = tb_t;
                res = res_t;
                res_norm = rn_t;
            } else {
                return Err(format!(
                    "solvepde: nonlinear solver failed to decrease residual below {res_norm:.3e} (MinStep = {:.3e})",
                    opts.min_step
                ));
            }
        }

        if res_norm <= opts.residual_tolerance {
            return Ok(u);
        }
    }

    Err(format!(
        "solvepde: nonlinear solver did not converge in {} iterations (residual norm {res_norm:.3e} > ResidualTolerance {:.3e})",
        opts.max_iterations, opts.residual_tolerance
    ))
}
