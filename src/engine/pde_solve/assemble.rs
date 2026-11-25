//! Reusable 2-D scalar finite-element assembly (K, M, F), Neumann/Robin
//! boundary assembly, strong Dirichlet elimination, and sparse linear solve.
//!
//! Designed to be shared by stationary (`solvepde`), time-dependent, and
//! eigenvalue solvers in Batch 8b.
use super::quadrature::{
    EDGE_QUAD_3, TRI_QUAD_P1, TRI_QUAD_P2, edge_p1_shape, edge_p2_shape, p1_jacobian_and_grads,
    p1_shape, p2_jacobian_and_grads, p2_shape,
};
use crate::engine::{Engine, Execution, Value, pde_geometry::Geometry2, pde_mesh::FeMesh};
use crate::sparse::SparseMatrix;
use crate::{Array, Matrix, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// Assembled domain finite-element system for a scalar 2-D PDE:
/// - `k`: stiffness matrix `∫ (∇φ_i)^T (c ∇φ_j) dΩ`
/// - `m`: reaction/mass matrix `∫ a φ_i φ_j dΩ`
/// - `f`: load vector `∫ f φ_i dΩ`
pub(crate) struct FemSystem {
    pub(crate) k: SparseMatrix,
    pub(crate) m: SparseMatrix,
    pub(crate) f: Vec<f64>,
}

/// A CCW exterior boundary facet of a triangle, mapped to its 1-based geometry edge.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BoundaryFacet {
    pub(crate) edge_id: usize,
    pub(crate) n0: usize,
    pub(crate) n1: usize,
    pub(crate) mid: Option<usize>,
}

/// A domain quadrature evaluation point.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DomainQuadPoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) subdomain: usize,
}

/// A boundary quadrature evaluation point (with outward unit normal).
#[derive(Clone, Copy, Debug)]
pub(crate) struct EdgeQuadPoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) nx: f64,
    pub(crate) ny: f64,
    pub(crate) edge_id: usize,
}

/// Reduced linear system after strong Dirichlet elimination.
pub(crate) struct ReducedSystem {
    pub(crate) a_ii: SparseMatrix,
    pub(crate) b_eff: Vec<f64>,
    pub(crate) free_dofs: Vec<usize>,
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Execution cancelled".into())
    } else {
        Ok(())
    }
}

/// Build a per-element 1-based face ID lookup from `FeMesh::face_elements`.
pub(crate) fn element_faces(mesh: &FeMesh) -> Vec<usize> {
    let mut tri_face = vec![0usize; mesh.tris.len()];
    for (fi, elems) in mesh.face_elements.iter().enumerate() {
        let face_id = fi + 1;
        for &e1 in elems {
            if e1 >= 1 && e1 <= tri_face.len() {
                tri_face[e1 - 1] = face_id;
            }
        }
    }
    tri_face
}

/// Extract all CCW exterior boundary facets of `mesh` and map each to its
/// 1-based geometry edge ID.
pub(crate) fn extract_boundary_facets(mesh: &FeMesh, geom: &Geometry2) -> Vec<BoundaryFacet> {
    // Count occurrences of each unordered corner edge; exterior facets occur once.
    // Value: Some((n0, n1, mid)) on first sight, None once seen twice (interior).
    #[allow(clippy::type_complexity)]
    let mut facet_map: HashMap<(usize, usize), Option<(usize, usize, Option<usize>)>> =
        HashMap::with_capacity(mesh.tris.len() * 2);
    for (t, &tri) in mesh.tris.iter().enumerate() {
        let mids = mesh.mid.as_ref().map(|m| m[t]);
        let local_edges = [
            (tri[0], tri[1], mids.map(|m| m[0])),
            (tri[1], tri[2], mids.map(|m| m[1])),
            (tri[2], tri[0], mids.map(|m| m[2])),
        ];
        for (a, b, mid) in local_edges {
            let key = if a < b { (a, b) } else { (b, a) };
            facet_map
                .entry(key)
                .and_modify(|slot| *slot = None)
                .or_insert(Some((a, b, mid)));
        }
    }

    // Build reverse map: 0-based node -> sorted list of 1-based geometry edge IDs.
    let mut node_to_edges: Vec<Vec<usize>> = vec![Vec::new(); mesh.nodes.len()];
    for (ei, nodes) in mesh.edge_nodes.iter().enumerate() {
        let edge_id = ei + 1;
        for &n1 in nodes {
            if n1 >= 1 && n1 <= mesh.nodes.len() {
                node_to_edges[n1 - 1].push(edge_id);
            }
        }
    }

    let mut facets = Vec::new();
    for (n0, n1, mid) in facet_map.into_values().flatten() {
        let edge_id = match mid {
            Some(m) if !node_to_edges[m].is_empty() => node_to_edges[m][0],
            _ => {
                // Intersect geometry edges of n0 and n1.
                let e0 = &node_to_edges[n0];
                let e1 = &node_to_edges[n1];
                let mut candidates: Vec<usize> = e0
                    .iter()
                    .copied()
                    .filter(|id| e1.binary_search(id).is_ok())
                    .collect();
                if candidates.len() == 1 {
                    candidates[0]
                } else {
                    if candidates.is_empty() {
                        candidates = (1..=geom.edges.len()).collect();
                    }
                    // Disambiguate by closest geometry edge to facet midpoint.
                    let p0 = mesh.nodes[n0];
                    let p1 = mesh.nodes[n1];
                    let pm = match mid {
                        Some(m) => mesh.nodes[m],
                        None => [0.5 * (p0[0] + p1[0]), 0.5 * (p0[1] + p1[1])],
                    };
                    closest_geometry_edge(geom, &candidates, pm)
                }
            }
        };
        facets.push(BoundaryFacet {
            edge_id,
            n0,
            n1,
            mid,
        });
    }
    // Deterministic order for reproducible floating-point sums.
    facets.sort_unstable_by_key(|f| (f.edge_id, f.n0, f.n1));
    facets
}

fn closest_geometry_edge(geom: &Geometry2, candidates: &[usize], p: [f64; 2]) -> usize {
    let mut best_id = candidates[0];
    let mut best_d2 = f64::INFINITY;
    for &eid in candidates {
        let edge = &geom.edges[eid - 1];
        for k in 0..=16 {
            let q = edge.point_at(k as f64 / 16.0);
            let dx = q[0] - p[0];
            let dy = q[1] - p[1];
            let d2 = dx * dx + dy * dy;
            if d2 < best_d2 {
                best_d2 = d2;
                best_id = eid;
            }
        }
    }
    best_id
}

/// Collect all domain quadrature points in element-major order.
/// Returns `(points, nq_per_element)`.
#[allow(clippy::needless_range_loop)]
pub(crate) fn collect_domain_quad_points(
    mesh: &FeMesh,
    tri_face: &[usize],
    cancel: &AtomicBool,
) -> Result<(Vec<DomainQuadPoint>, usize)> {
    let n_elems = mesh.tris.len();
    if mesh.quadratic {
        let mids = mesh
            .mid
            .as_ref()
            .ok_or("solvepde: quadratic mesh is missing midside nodes")?;
        let nq = TRI_QUAD_P2.len();
        let mut pts = Vec::with_capacity(n_elems * nq);
        for t in 0..n_elems {
            if t % 4096 == 0 {
                check_cancel(cancel)?;
            }
            let c = mesh.tris[t];
            let m = mids[t];
            let coords = [
                mesh.nodes[c[0]],
                mesh.nodes[c[1]],
                mesh.nodes[c[2]],
                mesh.nodes[m[0]],
                mesh.nodes[m[1]],
                mesh.nodes[m[2]],
            ];
            let sub = tri_face[t];
            for qp in &TRI_QUAD_P2 {
                let (n, _, _) = p2_shape(qp.xi, qp.eta);
                let mut x = 0.0;
                let mut y = 0.0;
                for a in 0..6 {
                    x += n[a] * coords[a][0];
                    y += n[a] * coords[a][1];
                }
                pts.push(DomainQuadPoint {
                    x,
                    y,
                    subdomain: sub,
                });
            }
        }
        Ok((pts, nq))
    } else {
        let nq = TRI_QUAD_P1.len();
        let mut pts = Vec::with_capacity(n_elems * nq);
        for t in 0..n_elems {
            if t % 4096 == 0 {
                check_cancel(cancel)?;
            }
            let c = mesh.tris[t];
            let p0 = mesh.nodes[c[0]];
            let p1 = mesh.nodes[c[1]];
            let p2 = mesh.nodes[c[2]];
            let sub = tri_face[t];
            for qp in &TRI_QUAD_P1 {
                let n = p1_shape(qp.xi, qp.eta);
                let x = n[0] * p0[0] + n[1] * p1[0] + n[2] * p2[0];
                let y = n[0] * p0[1] + n[1] * p1[1] + n[2] * p2[1];
                pts.push(DomainQuadPoint {
                    x,
                    y,
                    subdomain: sub,
                });
            }
        }
        Ok((pts, nq))
    }
}

/// Assemble sparse `K` (from `c`), sparse `M` (from `a`), and load vector `F` (from `f`)
/// for a scalar 2-D PDE on `mesh`.
///
/// `c_vals`, `a_vals`, and `f_vals` hold one value per domain quadrature point
/// in element-major order matching `collect_domain_quad_points`.
/// Each `c_vals[q]` is `[c11, c21, c12, c22]`.
#[allow(clippy::needless_range_loop)]
pub(crate) fn assemble_kmf(
    mesh: &FeMesh,
    c_vals: &[[f64; 4]],
    a_vals: &[f64],
    f_vals: &[f64],
    cancel: &AtomicBool,
) -> Result<FemSystem> {
    let n_nodes = mesh.nodes.len();
    let n_elems = mesh.tris.len();
    let mut f_vec = vec![0.0; n_nodes];

    if mesh.quadratic {
        let mids = mesh
            .mid
            .as_ref()
            .ok_or("solvepde: quadratic mesh is missing midside nodes")?;
        let nq = TRI_QUAD_P2.len();
        debug_assert_eq!(c_vals.len(), n_elems * nq);
        debug_assert_eq!(a_vals.len(), n_elems * nq);
        debug_assert_eq!(f_vals.len(), n_elems * nq);

        // Precompute shape functions at all 7 quadrature points.
        let shapes: Vec<([f64; 6], [f64; 6], [f64; 6])> = TRI_QUAD_P2
            .iter()
            .map(|qp| p2_shape(qp.xi, qp.eta))
            .collect();

        let any_a = a_vals.iter().any(|&v| v != 0.0);
        let mut k_triplets: Vec<(usize, usize, f64)> = Vec::with_capacity(n_elems * 36);
        let mut m_triplets: Vec<(usize, usize, f64)> = if any_a {
            Vec::with_capacity(n_elems * 36)
        } else {
            Vec::new()
        };

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
            let mut ke = [[0.0; 6]; 6];
            let mut me = [[0.0; 6]; 6];
            let mut fe = [0.0; 6];

            for (q, qp) in TRI_QUAD_P2.iter().enumerate() {
                let idx = t * nq + q;
                let (n, dxi, deta) = &shapes[q];
                let (det, dnx, dny) = p2_jacobian_and_grads(&coords, dxi, deta)?;
                let w = qp.w * det;
                let [c11, c21, c12, c22] = c_vals[idx];
                let av = a_vals[idx];
                let fv = f_vals[idx];

                for i in 0..6 {
                    fe[i] += w * fv * n[i];
                    let gx_i = dnx[i];
                    let gy_i = dny[i];
                    for j in 0..6 {
                        let flux_x = c11 * dnx[j] + c12 * dny[j];
                        let flux_y = c21 * dnx[j] + c22 * dny[j];
                        ke[i][j] += w * (gx_i * flux_x + gy_i * flux_y);
                    }
                    if av != 0.0 {
                        let wa_ni = w * av * n[i];
                        for j in 0..6 {
                            me[i][j] += wa_ni * n[j];
                        }
                    }
                }
            }

            for i in 0..6 {
                let gi = dofs[i];
                f_vec[gi] += fe[i];
                for j in 0..6 {
                    let gj = dofs[j];
                    if ke[i][j] != 0.0 {
                        k_triplets.push((gi, gj, ke[i][j]));
                    }
                    if any_a && me[i][j] != 0.0 {
                        m_triplets.push((gi, gj, me[i][j]));
                    }
                }
            }
        }

        let k = SparseMatrix::from_triplets(n_nodes, n_nodes, &k_triplets, false, cancel)?;
        let m = if any_a {
            SparseMatrix::from_triplets(n_nodes, n_nodes, &m_triplets, false, cancel)?
        } else {
            SparseMatrix::zeros(n_nodes, n_nodes, false)?
        };
        Ok(FemSystem { k, m, f: f_vec })
    } else {
        let nq = TRI_QUAD_P1.len();
        debug_assert_eq!(c_vals.len(), n_elems * nq);
        debug_assert_eq!(a_vals.len(), n_elems * nq);
        debug_assert_eq!(f_vals.len(), n_elems * nq);

        let shapes: Vec<[f64; 3]> = TRI_QUAD_P1
            .iter()
            .map(|qp| p1_shape(qp.xi, qp.eta))
            .collect();
        let any_a = a_vals.iter().any(|&v| v != 0.0);
        let mut k_triplets: Vec<(usize, usize, f64)> = Vec::with_capacity(n_elems * 9);
        let mut m_triplets: Vec<(usize, usize, f64)> = if any_a {
            Vec::with_capacity(n_elems * 9)
        } else {
            Vec::new()
        };

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
            let (det, dnx, dny) = p1_jacobian_and_grads(&coords)?;
            let mut ke = [[0.0; 3]; 3];
            let mut me = [[0.0; 3]; 3];
            let mut fe = [0.0; 3];

            for (q, qp) in TRI_QUAD_P1.iter().enumerate() {
                let idx = t * nq + q;
                let n = &shapes[q];
                let w = qp.w * det;
                let [c11, c21, c12, c22] = c_vals[idx];
                let av = a_vals[idx];
                let fv = f_vals[idx];

                for i in 0..3 {
                    fe[i] += w * fv * n[i];
                    let gx_i = dnx[i];
                    let gy_i = dny[i];
                    for j in 0..3 {
                        let flux_x = c11 * dnx[j] + c12 * dny[j];
                        let flux_y = c21 * dnx[j] + c22 * dny[j];
                        ke[i][j] += w * (gx_i * flux_x + gy_i * flux_y);
                    }
                    if av != 0.0 {
                        let wa_ni = w * av * n[i];
                        for j in 0..3 {
                            me[i][j] += wa_ni * n[j];
                        }
                    }
                }
            }

            for i in 0..3 {
                let gi = dofs[i];
                f_vec[gi] += fe[i];
                for j in 0..3 {
                    let gj = dofs[j];
                    if ke[i][j] != 0.0 {
                        k_triplets.push((gi, gj, ke[i][j]));
                    }
                    if any_a && me[i][j] != 0.0 {
                        m_triplets.push((gi, gj, me[i][j]));
                    }
                }
            }
        }

        let k = SparseMatrix::from_triplets(n_nodes, n_nodes, &k_triplets, false, cancel)?;
        let m = if any_a {
            SparseMatrix::from_triplets(n_nodes, n_nodes, &m_triplets, false, cancel)?
        } else {
            SparseMatrix::zeros(n_nodes, n_nodes, false)?
        };
        Ok(FemSystem { k, m, f: f_vec })
    }
}

/// Collect boundary quadrature points (with outward unit normals) for a slice of
/// boundary facets. Returns `(points, nq_per_facet)`.
pub(crate) fn collect_edge_quad_points(
    mesh: &FeMesh,
    facets: &[BoundaryFacet],
) -> Result<(Vec<EdgeQuadPoint>, usize)> {
    let nq = EDGE_QUAD_3.len();
    let mut pts = Vec::with_capacity(facets.len() * nq);
    for f in facets {
        let p0 = mesh.nodes[f.n0];
        let p1 = mesh.nodes[f.n1];
        match f.mid {
            Some(mid) => {
                let pm = mesh.nodes[mid];
                for &(t, _) in &EDGE_QUAD_3 {
                    let (n, dt) = edge_p2_shape(t);
                    let x = n[0] * p0[0] + n[1] * p1[0] + n[2] * pm[0];
                    let y = n[0] * p0[1] + n[1] * p1[1] + n[2] * pm[1];
                    let tx = dt[0] * p0[0] + dt[1] * p1[0] + dt[2] * pm[0];
                    let ty = dt[0] * p0[1] + dt[1] * p1[1] + dt[2] * pm[1];
                    let len = tx.hypot(ty);
                    if !(len.is_finite() && len > 0.0) {
                        return Err("solvepde: degenerate boundary edge".into());
                    }
                    pts.push(EdgeQuadPoint {
                        x,
                        y,
                        nx: ty / len,
                        ny: -tx / len,
                        edge_id: f.edge_id,
                    });
                }
            }
            None => {
                let tx = p1[0] - p0[0];
                let ty = p1[1] - p0[1];
                let len = tx.hypot(ty);
                if !(len.is_finite() && len > 0.0) {
                    return Err("solvepde: degenerate boundary edge".into());
                }
                let nx = ty / len;
                let ny = -tx / len;
                for &(t, _) in &EDGE_QUAD_3 {
                    let (n, _) = edge_p1_shape(t);
                    pts.push(EdgeQuadPoint {
                        x: n[0] * p0[0] + n[1] * p1[0],
                        y: n[0] * p0[1] + n[1] * p1[1],
                        nx,
                        ny,
                        edge_id: f.edge_id,
                    });
                }
            }
        }
    }
    Ok((pts, nq))
}

/// Assemble generalized Neumann / Robin boundary terms `Q` (from `q`) and `G` (from `g`).
pub(crate) fn assemble_neumann(
    mesh: &FeMesh,
    facets: &[BoundaryFacet],
    q_vals: &[f64],
    g_vals: &[f64],
    cancel: &AtomicBool,
) -> Result<(SparseMatrix, Vec<f64>)> {
    let n_nodes = mesh.nodes.len();
    let nq = EDGE_QUAD_3.len();
    let mut g_vec = vec![0.0; n_nodes];
    let any_q = q_vals.iter().any(|&v| v != 0.0);
    let mut q_triplets: Vec<(usize, usize, f64)> = Vec::new();

    for (fi, f) in facets.iter().enumerate() {
        if fi % 1024 == 0 {
            check_cancel(cancel)?;
        }
        let p0 = mesh.nodes[f.n0];
        let p1 = mesh.nodes[f.n1];
        match f.mid {
            Some(mid) => {
                let pm = mesh.nodes[mid];
                let dofs = [f.n0, f.n1, mid];
                let mut qe = [[0.0; 3]; 3];
                let mut ge = [0.0; 3];
                for (q, &(t, wt)) in EDGE_QUAD_3.iter().enumerate() {
                    let idx = fi * nq + q;
                    let (n, dt) = edge_p2_shape(t);
                    let tx = dt[0] * p0[0] + dt[1] * p1[0] + dt[2] * pm[0];
                    let ty = dt[0] * p0[1] + dt[1] * p1[1] + dt[2] * pm[1];
                    let ds = tx.hypot(ty) * wt;
                    let qv = q_vals[idx];
                    let gv = g_vals[idx];
                    for i in 0..3 {
                        ge[i] += ds * gv * n[i];
                        if qv != 0.0 {
                            let wq_ni = ds * qv * n[i];
                            for j in 0..3 {
                                qe[i][j] += wq_ni * n[j];
                            }
                        }
                    }
                }
                for i in 0..3 {
                    g_vec[dofs[i]] += ge[i];
                    if any_q {
                        for j in 0..3 {
                            if qe[i][j] != 0.0 {
                                q_triplets.push((dofs[i], dofs[j], qe[i][j]));
                            }
                        }
                    }
                }
            }
            None => {
                let dofs = [f.n0, f.n1];
                let len = (p1[0] - p0[0]).hypot(p1[1] - p0[1]);
                let mut qe = [[0.0; 2]; 2];
                let mut ge = [0.0; 2];
                for (q, &(t, wt)) in EDGE_QUAD_3.iter().enumerate() {
                    let idx = fi * nq + q;
                    let (n, _) = edge_p1_shape(t);
                    let ds = len * wt;
                    let qv = q_vals[idx];
                    let gv = g_vals[idx];
                    for i in 0..2 {
                        ge[i] += ds * gv * n[i];
                        if qv != 0.0 {
                            let wq_ni = ds * qv * n[i];
                            for j in 0..2 {
                                qe[i][j] += wq_ni * n[j];
                            }
                        }
                    }
                }
                for i in 0..2 {
                    g_vec[dofs[i]] += ge[i];
                    if any_q {
                        for j in 0..2 {
                            if qe[i][j] != 0.0 {
                                q_triplets.push((dofs[i], dofs[j], qe[i][j]));
                            }
                        }
                    }
                }
            }
        }
    }

    let q_mat = if any_q {
        SparseMatrix::from_triplets(n_nodes, n_nodes, &q_triplets, false, cancel)?
    } else {
        SparseMatrix::zeros(n_nodes, n_nodes, false)?
    };
    Ok((q_mat, g_vec))
}

/// Apply strong Dirichlet boundary conditions by elimination:
/// partitions DOFs into free `I` and constrained `D`, computes
/// `b_eff = b_I - A_ID * u_D` and extracts `A_II`.
pub(crate) fn apply_dirichlet_elimination(
    a: &SparseMatrix,
    b: &[f64],
    dirichlet_mask: &[bool],
    dirichlet_vals: &[f64],
    cancel: &AtomicBool,
) -> Result<ReducedSystem> {
    let n = a.rows();
    debug_assert_eq!(a.cols(), n);
    debug_assert_eq!(b.len(), n);
    debug_assert_eq!(dirichlet_mask.len(), n);
    debug_assert_eq!(dirichlet_vals.len(), n);

    let mut free_dofs = Vec::with_capacity(n);
    let mut node_to_free = vec![usize::MAX; n];
    for i in 0..n {
        if !dirichlet_mask[i] {
            node_to_free[i] = free_dofs.len();
            free_dofs.push(i);
        }
    }
    let n_free = free_dofs.len();
    let mut b_eff: Vec<f64> = free_dofs.iter().map(|&i| b[i]).collect();

    if n_free == 0 {
        return Ok(ReducedSystem {
            a_ii: SparseMatrix::zeros(0, 0, false)?,
            b_eff,
            free_dofs,
        });
    }

    // Single pass over CSC columns of A:
    // - If column c is Dirichlet: subtract A(r, c) * u_D[c] from b_eff[free_r] for free rows r.
    // - If column c is free: emit (free_r, free_c, val) into A_II for free rows r.
    let rows = a.rowidx();
    let mut colptr = Vec::with_capacity(n_free + 1);
    let mut rowidx = Vec::with_capacity(a.nnz());
    let mut vals = Vec::with_capacity(a.nnz());
    colptr.push(0);

    for c in 0..n {
        if c % 4096 == 0 {
            check_cancel(cancel)?;
        }
        if dirichlet_mask[c] {
            let ud = dirichlet_vals[c];
            if ud != 0.0 {
                for k in a.column(c) {
                    let r = rows[k];
                    let fr = node_to_free[r];
                    if fr != usize::MAX {
                        b_eff[fr] -= a.value(k) * ud;
                    }
                }
            }
        } else {
            for k in a.column(c) {
                let r = rows[k];
                let fr = node_to_free[r];
                if fr != usize::MAX {
                    let v = a.value(k);
                    if v != 0.0 {
                        rowidx.push(fr);
                        vals.push(v);
                    }
                }
            }
            colptr.push(rowidx.len());
        }
    }

    let a_ii = SparseMatrix::from_parts(
        n_free,
        n_free,
        colptr,
        rowidx,
        crate::sparse::SparseValues::Double(vals),
    )?;
    Ok(ReducedSystem {
        a_ii,
        b_eff,
        free_dofs,
    })
}

/// Solve `A * x = b` using the engine's sparse `mldivide` path (`sparse_solve.rs`).
pub(crate) fn solve_sparse_system(
    e: &mut Engine,
    a: &SparseMatrix,
    b: &[f64],
    out: &mut Execution,
) -> Result<Vec<f64>> {
    let n = a.rows();
    if n == 0 {
        return Ok(Vec::new());
    }
    if n == 1 {
        let diag = a.get(0, 0);
        if !diag.is_finite() || diag == 0.0 {
            return Err(
                "solvepde: stiffness matrix is singular; check boundary conditions and coefficients"
                    .into(),
            );
        }
        return Ok(vec![b[0] / diag]);
    }
    let a_val = Value::Container(Array::Sparse(a.clone()));
    let b_val = Value::Matrix(Matrix::from_column_slice(n, 1, b)?);
    match super::super::sparse_solve::divide(e, true, &a_val, &b_val, out)? {
        Some(Value::Matrix(x)) => {
            if x.data().iter().any(|v| !v.is_finite()) {
                return Err(
                    "solvepde: linear solve produced non-finite values; check boundary conditions and coefficients"
                        .into(),
                );
            }
            Ok(x.data().to_vec())
        }
        Some(_) => Err("solvepde: unexpected sparse solver output".into()),
        None => Err(
            "solvepde: stiffness matrix is singular or ill-conditioned; check boundary conditions and coefficients"
                .into(),
        ),
    }
}

/// Reconstruct the full nodal solution vector from free and Dirichlet DOFs.
pub(crate) fn reconstruct_solution(
    n_nodes: usize,
    free_dofs: &[usize],
    u_free: &[f64],
    dirichlet_mask: &[bool],
    dirichlet_vals: &[f64],
) -> Vec<f64> {
    let mut u = vec![0.0; n_nodes];
    for (&dof, &val) in free_dofs.iter().zip(u_free.iter()) {
        u[dof] = val;
    }
    for i in 0..n_nodes {
        if dirichlet_mask[i] {
            u[i] = dirichlet_vals[i];
        }
    }
    u
}

/// Assemble sparse mass matrix `∫ weight φ_i φ_j dΩ` on `mesh`.
///
/// The weight is the reaction coefficient `a` of [`assemble_kmf`] (`c = 0`, `f = 0`).
pub(crate) fn assemble_mass(
    mesh: &FeMesh,
    weight_vals: &[f64],
    cancel: &AtomicBool,
) -> Result<SparseMatrix> {
    let n_nodes = mesh.nodes.len();
    if !weight_vals.iter().any(|&v| v != 0.0) {
        return SparseMatrix::zeros(n_nodes, n_nodes, false);
    }
    let n = weight_vals.len();
    let c_zero = vec![[0.0; 4]; n];
    let f_zero = vec![0.0; n];
    Ok(assemble_kmf(mesh, &c_zero, weight_vals, &f_zero, cancel)?.m)
}

/// Assemble domain load vector `∫ f φ_i dΩ` on `mesh`.
///
/// Calls [`assemble_kmf`] with `c = 0` and `a = 0`.
pub(crate) fn assemble_load(
    mesh: &FeMesh,
    f_vals: &[f64],
    cancel: &AtomicBool,
) -> Result<Vec<f64>> {
    let n_nodes = mesh.nodes.len();
    if !f_vals.iter().any(|&v| v != 0.0) {
        return Ok(vec![0.0; n_nodes]);
    }
    let n = f_vals.len();
    let c_zero = vec![[0.0; 4]; n];
    let a_zero = vec![0.0; n];
    Ok(assemble_kmf(mesh, &c_zero, &a_zero, f_vals, cancel)?.f)
}

/// Partition a square sparse matrix `A` into `(A_II, A_ID)` given a free/Dirichlet DOF map.
#[allow(clippy::needless_range_loop)]
pub(crate) fn partition_matrix(
    a: &SparseMatrix,
    dirichlet_mask: &[bool],
    node_to_free: &[usize],
    n_free: usize,
    n_dir: usize,
    cancel: &AtomicBool,
) -> Result<(SparseMatrix, SparseMatrix)> {
    let n = a.rows();
    debug_assert_eq!(a.cols(), n);
    let rows = a.rowidx();

    let mut ii_colptr = Vec::with_capacity(n_free + 1);
    let mut ii_rowidx = Vec::new();
    let mut ii_vals = Vec::new();
    ii_colptr.push(0);

    let mut id_colptr = Vec::with_capacity(n_dir + 1);
    let mut id_rowidx = Vec::new();
    let mut id_vals = Vec::new();
    id_colptr.push(0);

    for c in 0..n {
        if c % 4096 == 0 {
            check_cancel(cancel)?;
        }
        if dirichlet_mask[c] {
            for k in a.column(c) {
                let fr = node_to_free[rows[k]];
                if fr != usize::MAX {
                    let v = a.value(k);
                    if v != 0.0 {
                        id_rowidx.push(fr);
                        id_vals.push(v);
                    }
                }
            }
            id_colptr.push(id_rowidx.len());
        } else {
            for k in a.column(c) {
                let fr = node_to_free[rows[k]];
                if fr != usize::MAX {
                    let v = a.value(k);
                    if v != 0.0 {
                        ii_rowidx.push(fr);
                        ii_vals.push(v);
                    }
                }
            }
            ii_colptr.push(ii_rowidx.len());
        }
    }

    let a_ii = SparseMatrix::from_parts(
        n_free,
        n_free,
        ii_colptr,
        ii_rowidx,
        crate::sparse::SparseValues::Double(ii_vals),
    )?;
    let a_id = SparseMatrix::from_parts(
        n_free,
        n_dir,
        id_colptr,
        id_rowidx,
        crate::sparse::SparseValues::Double(id_vals),
    )?;
    Ok((a_ii, a_id))
}

/// Sparse matrix-vector multiply `y += scale * A * x`.
pub(crate) fn sparse_matvec_add(a: &SparseMatrix, x: &[f64], scale: f64, y: &mut [f64]) {
    debug_assert_eq!(a.rows(), y.len());
    debug_assert_eq!(a.cols(), x.len());
    let rows = a.rowidx();
    for (c, &xc) in x.iter().enumerate() {
        if xc == 0.0 {
            continue;
        }
        let sxc = scale * xc;
        for k in a.column(c) {
            y[rows[k]] += a.value(k) * sxc;
        }
    }
}

/// Write `scale * A` into a dense column-major matrix `dst` of leading dimension `ld`
/// starting at block offset `(row_off, col_off)`.
pub(crate) fn sparse_scatter_dense(
    a: &SparseMatrix,
    scale: f64,
    dst: &mut [f64],
    ld: usize,
    row_off: usize,
    col_off: usize,
) {
    let rows = a.rowidx();
    for c in 0..a.cols() {
        let col_base = (col_off + c) * ld + row_off;
        for k in a.column(c) {
            dst[col_base + rows[k]] += scale * a.value(k);
        }
    }
}

// ---------------------------------------------------------------------------
// N-equation system assembly (component-major DOF: comp * n_nodes + node)

/// Assemble sparse `K` (from `c`), sparse `M` (from `a`), and load vector `F` (from `f`)
/// for an `n_comp`-equation 2-D PDE system on `mesh`.
///
/// - `c_vals`: `n_dom` blocks of column-major `(2*n_comp) x (2*n_comp)` tensors.
/// - `a_vals`: `n_dom` blocks of column-major `n_comp x n_comp` matrices.
/// - `f_vals`: `n_dom` blocks of length-`n_comp` vectors.
#[allow(clippy::needless_range_loop)]
pub(crate) fn assemble_kmf_sys(
    mesh: &FeMesh,
    n_comp: usize,
    c_vals: &[f64],
    a_vals: &[f64],
    f_vals: &[f64],
    cancel: &AtomicBool,
) -> Result<FemSystem> {
    if n_comp == 1 {
        let n_dom = f_vals.len();
        let mut c4 = Vec::with_capacity(n_dom);
        for q in 0..n_dom {
            let b = q * 4;
            c4.push([c_vals[b], c_vals[b + 1], c_vals[b + 2], c_vals[b + 3]]);
        }
        return assemble_kmf(mesh, &c4, a_vals, f_vals, cancel);
    }

    let n_nodes = mesh.nodes.len();
    let n_dofs = n_nodes * n_comp;
    let n_elems = mesh.tris.len();
    let m2 = 2 * n_comp;
    let c_stride = m2 * m2;
    let a_stride = n_comp * n_comp;
    let any_c = c_vals.iter().any(|&v| v != 0.0);
    let any_a = a_vals.iter().any(|&v| v != 0.0);

    let mut f_vec = vec![0.0; n_dofs];
    let mut k_triplets: Vec<(usize, usize, f64)> = Vec::new();
    let mut m_triplets: Vec<(usize, usize, f64)> = Vec::new();

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
            let nd_loc = 6 * n_comp;
            let mut ke = if any_c {
                vec![0.0; nd_loc * nd_loc]
            } else {
                Vec::new()
            };
            let mut me = if any_a {
                vec![0.0; nd_loc * nd_loc]
            } else {
                Vec::new()
            };
            let mut fe = vec![0.0; nd_loc];

            for (q, qp) in TRI_QUAD_P2.iter().enumerate() {
                let idx = t * nq + q;
                let (n, dxi, deta) = &shapes[q];
                let (det, dnx, dny) = p2_jacobian_and_grads(&coords, dxi, deta)?;
                let w = qp.w * det;
                let c_blk = &c_vals[idx * c_stride..(idx + 1) * c_stride];
                let a_blk = &a_vals[idx * a_stride..(idx + 1) * a_stride];
                let f_blk = &f_vals[idx * n_comp..(idx + 1) * n_comp];

                for ci in 0..n_comp {
                    let fv = f_blk[ci];
                    if fv != 0.0 {
                        let wf = w * fv;
                        for i in 0..6 {
                            fe[ci * 6 + i] += wf * n[i];
                        }
                    }
                    for cj in 0..n_comp {
                        if any_c {
                            let r0 = 2 * ci;
                            let c0 = 2 * cj;
                            let c11 = c_blk[c0 * m2 + r0];
                            let c21 = c_blk[c0 * m2 + r0 + 1];
                            let c12 = c_blk[(c0 + 1) * m2 + r0];
                            let c22 = c_blk[(c0 + 1) * m2 + r0 + 1];
                            if c11 != 0.0 || c21 != 0.0 || c12 != 0.0 || c22 != 0.0 {
                                for i in 0..6 {
                                    let row = ci * 6 + i;
                                    let gxi = dnx[i];
                                    let gyi = dny[i];
                                    for j in 0..6 {
                                        let col = cj * 6 + j;
                                        let fx = c11 * dnx[j] + c12 * dny[j];
                                        let fy = c21 * dnx[j] + c22 * dny[j];
                                        ke[row * nd_loc + col] += w * (gxi * fx + gyi * fy);
                                    }
                                }
                            }
                        }
                        if any_a {
                            let av = a_blk[cj * n_comp + ci];
                            if av != 0.0 {
                                let wa = w * av;
                                for i in 0..6 {
                                    let row = ci * 6 + i;
                                    let wani = wa * n[i];
                                    for j in 0..6 {
                                        let col = cj * 6 + j;
                                        me[row * nd_loc + col] += wani * n[j];
                                    }
                                }
                            }
                        }
                    }
                }
            }

            for ci in 0..n_comp {
                for i in 0..6 {
                    let row = ci * 6 + i;
                    let gi = ci * n_nodes + dofs[i];
                    f_vec[gi] += fe[row];
                    for cj in 0..n_comp {
                        for j in 0..6 {
                            let col = cj * 6 + j;
                            let gj = cj * n_nodes + dofs[j];
                            if any_c {
                                let kv = ke[row * nd_loc + col];
                                if kv != 0.0 {
                                    k_triplets.push((gi, gj, kv));
                                }
                            }
                            if any_a {
                                let mv = me[row * nd_loc + col];
                                if mv != 0.0 {
                                    m_triplets.push((gi, gj, mv));
                                }
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
            let nd_loc = 3 * n_comp;
            let mut ke = if any_c {
                vec![0.0; nd_loc * nd_loc]
            } else {
                Vec::new()
            };
            let mut me = if any_a {
                vec![0.0; nd_loc * nd_loc]
            } else {
                Vec::new()
            };
            let mut fe = vec![0.0; nd_loc];

            for (q, qp) in TRI_QUAD_P1.iter().enumerate() {
                let idx = t * nq + q;
                let n = &shapes[q];
                let w = qp.w * det;
                let c_blk = &c_vals[idx * c_stride..(idx + 1) * c_stride];
                let a_blk = &a_vals[idx * a_stride..(idx + 1) * a_stride];
                let f_blk = &f_vals[idx * n_comp..(idx + 1) * n_comp];

                for ci in 0..n_comp {
                    let fv = f_blk[ci];
                    if fv != 0.0 {
                        let wf = w * fv;
                        for i in 0..3 {
                            fe[ci * 3 + i] += wf * n[i];
                        }
                    }
                    for cj in 0..n_comp {
                        if any_c {
                            let r0 = 2 * ci;
                            let c0 = 2 * cj;
                            let c11 = c_blk[c0 * m2 + r0];
                            let c21 = c_blk[c0 * m2 + r0 + 1];
                            let c12 = c_blk[(c0 + 1) * m2 + r0];
                            let c22 = c_blk[(c0 + 1) * m2 + r0 + 1];
                            if c11 != 0.0 || c21 != 0.0 || c12 != 0.0 || c22 != 0.0 {
                                for i in 0..3 {
                                    let row = ci * 3 + i;
                                    let gxi = dnx[i];
                                    let gyi = dny[i];
                                    for j in 0..3 {
                                        let col = cj * 3 + j;
                                        let fx = c11 * dnx[j] + c12 * dny[j];
                                        let fy = c21 * dnx[j] + c22 * dny[j];
                                        ke[row * nd_loc + col] += w * (gxi * fx + gyi * fy);
                                    }
                                }
                            }
                        }
                        if any_a {
                            let av = a_blk[cj * n_comp + ci];
                            if av != 0.0 {
                                let wa = w * av;
                                for i in 0..3 {
                                    let row = ci * 3 + i;
                                    let wani = wa * n[i];
                                    for j in 0..3 {
                                        let col = cj * 3 + j;
                                        me[row * nd_loc + col] += wani * n[j];
                                    }
                                }
                            }
                        }
                    }
                }
            }

            for ci in 0..n_comp {
                for i in 0..3 {
                    let row = ci * 3 + i;
                    let gi = ci * n_nodes + dofs[i];
                    f_vec[gi] += fe[row];
                    for cj in 0..n_comp {
                        for j in 0..3 {
                            let col = cj * 3 + j;
                            let gj = cj * n_nodes + dofs[j];
                            if any_c {
                                let kv = ke[row * nd_loc + col];
                                if kv != 0.0 {
                                    k_triplets.push((gi, gj, kv));
                                }
                            }
                            if any_a {
                                let mv = me[row * nd_loc + col];
                                if mv != 0.0 {
                                    m_triplets.push((gi, gj, mv));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let k = if any_c {
        SparseMatrix::from_triplets(n_dofs, n_dofs, &k_triplets, false, cancel)?
    } else {
        SparseMatrix::zeros(n_dofs, n_dofs, false)?
    };
    let m = if any_a {
        SparseMatrix::from_triplets(n_dofs, n_dofs, &m_triplets, false, cancel)?
    } else {
        SparseMatrix::zeros(n_dofs, n_dofs, false)?
    };
    Ok(FemSystem { k, m, f: f_vec })
}

/// Assemble sparse mass matrix for an `n_comp`-equation system.
pub(crate) fn assemble_mass_sys(
    mesh: &FeMesh,
    n_comp: usize,
    weight_vals: &[f64],
    cancel: &AtomicBool,
) -> Result<SparseMatrix> {
    if n_comp == 1 {
        return assemble_mass(mesh, weight_vals, cancel);
    }
    let n_dofs = mesh.nodes.len() * n_comp;
    if !weight_vals.iter().any(|&v| v != 0.0) {
        return SparseMatrix::zeros(n_dofs, n_dofs, false);
    }
    let n_dom = weight_vals.len() / (n_comp * n_comp);
    let c_zero = vec![0.0; n_dom * 4 * n_comp * n_comp];
    let f_zero = vec![0.0; n_dom * n_comp];
    Ok(assemble_kmf_sys(mesh, n_comp, &c_zero, weight_vals, &f_zero, cancel)?.m)
}

/// Assemble domain load vector for an `n_comp`-equation system.
pub(crate) fn assemble_load_sys(
    mesh: &FeMesh,
    n_comp: usize,
    f_vals: &[f64],
    cancel: &AtomicBool,
) -> Result<Vec<f64>> {
    if n_comp == 1 {
        return assemble_load(mesh, f_vals, cancel);
    }
    let n_dofs = mesh.nodes.len() * n_comp;
    if !f_vals.iter().any(|&v| v != 0.0) {
        return Ok(vec![0.0; n_dofs]);
    }
    let n_dom = f_vals.len() / n_comp;
    let c_zero = vec![0.0; n_dom * 4 * n_comp * n_comp];
    let a_zero = vec![0.0; n_dom * n_comp * n_comp];
    Ok(assemble_kmf_sys(mesh, n_comp, &c_zero, &a_zero, f_vals, cancel)?.f)
}

/// Assemble generalized Neumann / Robin boundary terms `Q` and `G` for an `n_comp`-equation system.
///
/// - `q_vals`: `n_ep` blocks of column-major `n_comp x n_comp` matrices.
/// - `g_vals`: `n_ep` blocks of length-`n_comp` vectors.
pub(crate) fn assemble_neumann_sys(
    mesh: &FeMesh,
    n_comp: usize,
    facets: &[BoundaryFacet],
    q_vals: &[f64],
    g_vals: &[f64],
    cancel: &AtomicBool,
) -> Result<(SparseMatrix, Vec<f64>)> {
    if n_comp == 1 {
        return assemble_neumann(mesh, facets, q_vals, g_vals, cancel);
    }
    let n_nodes = mesh.nodes.len();
    let n_dofs = n_nodes * n_comp;
    let nq = EDGE_QUAD_3.len();
    let q_stride = n_comp * n_comp;
    let mut g_vec = vec![0.0; n_dofs];
    let any_q = q_vals.iter().any(|&v| v != 0.0);
    let mut q_triplets: Vec<(usize, usize, f64)> = Vec::new();

    for (fi, f) in facets.iter().enumerate() {
        if fi % 1024 == 0 {
            check_cancel(cancel)?;
        }
        let p0 = mesh.nodes[f.n0];
        let p1 = mesh.nodes[f.n1];
        match f.mid {
            Some(mid) => {
                let pm = mesh.nodes[mid];
                let dofs = [f.n0, f.n1, mid];
                let nd_loc = 3 * n_comp;
                let mut qe = if any_q {
                    vec![0.0; nd_loc * nd_loc]
                } else {
                    Vec::new()
                };
                let mut ge = vec![0.0; nd_loc];
                for (q, &(t, wt)) in EDGE_QUAD_3.iter().enumerate() {
                    let idx = fi * nq + q;
                    let (n, dt) = edge_p2_shape(t);
                    let tx = dt[0] * p0[0] + dt[1] * p1[0] + dt[2] * pm[0];
                    let ty = dt[0] * p0[1] + dt[1] * p1[1] + dt[2] * pm[1];
                    let ds = tx.hypot(ty) * wt;
                    let q_blk = &q_vals[idx * q_stride..(idx + 1) * q_stride];
                    let g_blk = &g_vals[idx * n_comp..(idx + 1) * n_comp];
                    for ci in 0..n_comp {
                        let gv = g_blk[ci];
                        if gv != 0.0 {
                            let wg = ds * gv;
                            for i in 0..3 {
                                ge[ci * 3 + i] += wg * n[i];
                            }
                        }
                        if any_q {
                            for cj in 0..n_comp {
                                let qv = q_blk[cj * n_comp + ci];
                                if qv != 0.0 {
                                    let wq = ds * qv;
                                    for i in 0..3 {
                                        let row = ci * 3 + i;
                                        let wqni = wq * n[i];
                                        for j in 0..3 {
                                            qe[row * nd_loc + cj * 3 + j] += wqni * n[j];
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                for ci in 0..n_comp {
                    for i in 0..3 {
                        let row = ci * 3 + i;
                        let gi = ci * n_nodes + dofs[i];
                        g_vec[gi] += ge[row];
                        if any_q {
                            for cj in 0..n_comp {
                                for j in 0..3 {
                                    let qv = qe[row * nd_loc + cj * 3 + j];
                                    if qv != 0.0 {
                                        q_triplets.push((gi, cj * n_nodes + dofs[j], qv));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            None => {
                let dofs = [f.n0, f.n1];
                let len = (p1[0] - p0[0]).hypot(p1[1] - p0[1]);
                let nd_loc = 2 * n_comp;
                let mut qe = if any_q {
                    vec![0.0; nd_loc * nd_loc]
                } else {
                    Vec::new()
                };
                let mut ge = vec![0.0; nd_loc];
                for (q, &(t, wt)) in EDGE_QUAD_3.iter().enumerate() {
                    let idx = fi * nq + q;
                    let (n, _) = edge_p1_shape(t);
                    let ds = len * wt;
                    let q_blk = &q_vals[idx * q_stride..(idx + 1) * q_stride];
                    let g_blk = &g_vals[idx * n_comp..(idx + 1) * n_comp];
                    for ci in 0..n_comp {
                        let gv = g_blk[ci];
                        if gv != 0.0 {
                            let wg = ds * gv;
                            for i in 0..2 {
                                ge[ci * 2 + i] += wg * n[i];
                            }
                        }
                        if any_q {
                            for cj in 0..n_comp {
                                let qv = q_blk[cj * n_comp + ci];
                                if qv != 0.0 {
                                    let wq = ds * qv;
                                    for i in 0..2 {
                                        let row = ci * 2 + i;
                                        let wqni = wq * n[i];
                                        for j in 0..2 {
                                            qe[row * nd_loc + cj * 2 + j] += wqni * n[j];
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                for ci in 0..n_comp {
                    for i in 0..2 {
                        let row = ci * 2 + i;
                        let gi = ci * n_nodes + dofs[i];
                        g_vec[gi] += ge[row];
                        if any_q {
                            for cj in 0..n_comp {
                                for j in 0..2 {
                                    let qv = qe[row * nd_loc + cj * 2 + j];
                                    if qv != 0.0 {
                                        q_triplets.push((gi, cj * n_nodes + dofs[j], qv));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let q_mat = if any_q {
        SparseMatrix::from_triplets(n_dofs, n_dofs, &q_triplets, false, cancel)?
    } else {
        SparseMatrix::zeros(n_dofs, n_dofs, false)?
    };
    Ok((q_mat, g_vec))
}
