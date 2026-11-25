//! Post-processing for 2-D PDE results:
//! - Area-weighted nodal gradient recovery (`XGradients`, `YGradients`)
//! - Spatial-grid point location on P1 (linear) and isoparametric P2 (quadratic) triangles
//! - `interpolateSolution(results, xq, yq[, iU][, iT])`
//! - `evaluateGradient(results, xq, yq[, iU][, iT])`
use super::quadrature::{
    TRI_QUAD_P2, p1_jacobian_and_grads, p1_shape, p2_jacobian_and_grads, p2_shape,
};
use crate::engine::{Engine, Value, objects, pde_mesh::FeMesh};
use crate::{Array, Matrix, Result};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Reference coordinates of the 6 P2 element nodes in MATLAB / `FeMesh` order.
const P2_NODE_REF: [(f64, f64); 6] = [
    (0.0, 0.0),
    (1.0, 0.0),
    (0.0, 1.0),
    (0.5, 0.0),
    (0.5, 0.5),
    (0.0, 0.5),
];

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Execution cancelled".into())
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Nodal recovered gradients (area-weighted average of element gradients)

/// Compute area-weighted nodal recovered gradients `(gx, gy)` for a single
/// scalar nodal field `u` of length `mesh.nodes.len()`.
#[allow(clippy::needless_range_loop)]
pub(crate) fn recover_nodal_gradients_slice(
    mesh: &FeMesh,
    u: &[f64],
    cancel: &AtomicBool,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let n_nodes = mesh.nodes.len();
    debug_assert_eq!(u.len(), n_nodes);
    let mut sum_gx = vec![0.0; n_nodes];
    let mut sum_gy = vec![0.0; n_nodes];
    let mut sum_area = vec![0.0; n_nodes];
    let n_elems = mesh.tris.len();

    if mesh.quadratic {
        let mids = mesh
            .mid
            .as_ref()
            .ok_or("solvepde: quadratic mesh is missing midside nodes")?;
        let quad_shapes: Vec<([f64; 6], [f64; 6], [f64; 6])> = TRI_QUAD_P2
            .iter()
            .map(|qp| p2_shape(qp.xi, qp.eta))
            .collect();
        let node_shapes: Vec<([f64; 6], [f64; 6], [f64; 6])> = P2_NODE_REF
            .iter()
            .map(|&(xi, eta)| p2_shape(xi, eta))
            .collect();

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
            let ue = [
                u[dofs[0]], u[dofs[1]], u[dofs[2]], u[dofs[3]], u[dofs[4]], u[dofs[5]],
            ];
            let mut area = 0.0;
            for (q, qp) in TRI_QUAD_P2.iter().enumerate() {
                let (_, dxi, deta) = &quad_shapes[q];
                let (det, _, _) = p2_jacobian_and_grads(&coords, dxi, deta)?;
                area += qp.w * det;
            }
            for a in 0..6 {
                let (_, dxi, deta) = &node_shapes[a];
                let (_, dnx, dny) = p2_jacobian_and_grads(&coords, dxi, deta)?;
                let mut gx = 0.0;
                let mut gy = 0.0;
                for b in 0..6 {
                    gx += dnx[b] * ue[b];
                    gy += dny[b] * ue[b];
                }
                let gi = dofs[a];
                sum_gx[gi] += area * gx;
                sum_gy[gi] += area * gy;
                sum_area[gi] += area;
            }
        }
    } else {
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
            let area = 0.5 * det;
            let gx = dnx[0] * u[dofs[0]] + dnx[1] * u[dofs[1]] + dnx[2] * u[dofs[2]];
            let gy = dny[0] * u[dofs[0]] + dny[1] * u[dofs[1]] + dny[2] * u[dofs[2]];
            for &gi in &dofs {
                sum_gx[gi] += area * gx;
                sum_gy[gi] += area * gy;
                sum_area[gi] += area;
            }
        }
    }

    for i in 0..n_nodes {
        if sum_area[i] > 0.0 {
            sum_gx[i] /= sum_area[i];
            sum_gy[i] /= sum_area[i];
        }
    }
    Ok((sum_gx, sum_gy))
}

/// Compute `(XGradients, YGradients)` with the exact same dimensions as `nodal_solution`.
/// Works for 2-D (`N x Ncomp` or `N x Nt`) and 3-D (`N x Ncomp x Nt`) column-major arrays
/// where every `N`-element column is one scalar nodal field.
pub(crate) fn compute_nodal_gradients(
    mesh: &FeMesh,
    nodal_solution: &Matrix,
    cancel: &AtomicBool,
) -> Result<(Matrix, Matrix)> {
    let n_nodes = mesh.nodes.len();
    let total = nodal_solution.len();
    if n_nodes == 0 || total == 0 {
        return Ok((nodal_solution.clone(), nodal_solution.clone()));
    }
    let n_slices = total / n_nodes;
    let data = nodal_solution.data();
    let mut gx_all = Vec::with_capacity(total);
    let mut gy_all = Vec::with_capacity(total);
    for s in 0..n_slices {
        let slice = &data[s * n_nodes..(s + 1) * n_nodes];
        let (gx, gy) = recover_nodal_gradients_slice(mesh, slice, cancel)?;
        gx_all.extend_from_slice(&gx);
        gy_all.extend_from_slice(&gy);
    }
    let dims = nodal_solution.dims();
    let gx_mat = if dims.len() > 2 {
        Matrix::from_column_vec(n_nodes, n_slices, gx_all)?.with_dims(&dims)?
    } else {
        Matrix::from_column_vec(nodal_solution.rows(), nodal_solution.cols(), gx_all)?
    };
    let gy_mat = if dims.len() > 2 {
        Matrix::from_column_vec(n_nodes, n_slices, gy_all)?.with_dims(&dims)?
    } else {
        Matrix::from_column_vec(nodal_solution.rows(), nodal_solution.cols(), gy_all)?
    };
    Ok((gx_mat, gy_mat))
}

// ---------------------------------------------------------------------------
// Spatial grid and point location for P1 and isoparametric P2 meshes

struct MeshLocator<'a> {
    mesh: &'a FeMesh,
    xmin: f64,
    ymin: f64,
    inv_dx: f64,
    inv_dy: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<usize>>,
}

impl<'a> MeshLocator<'a> {
    fn new(mesh: &'a FeMesh) -> Self {
        let mut xmin = f64::INFINITY;
        let mut xmax = f64::NEG_INFINITY;
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for &[x, y] in &mesh.nodes {
            xmin = xmin.min(x);
            xmax = xmax.max(x);
            ymin = ymin.min(y);
            ymax = ymax.max(y);
        }
        let span_x = (xmax - xmin).max(1e-12);
        let span_y = (ymax - ymin).max(1e-12);
        // Pad slightly to avoid boundary rounding issues.
        let pad_x = 1e-6 * span_x;
        let pad_y = 1e-6 * span_y;
        xmin -= pad_x;
        xmax += pad_x;
        ymin -= pad_y;
        ymax += pad_y;

        let n_elems = mesh.tris.len().max(1);
        let side = (n_elems as f64).sqrt().ceil().clamp(4.0, 256.0) as usize;
        let nx = side;
        let ny = side;
        let inv_dx = nx as f64 / (xmax - xmin);
        let inv_dy = ny as f64 / (ymax - ymin);
        let mut cells = vec![Vec::new(); nx * ny];

        let mids = mesh.mid.as_ref();
        for (t, &tri) in mesh.tris.iter().enumerate() {
            let p0 = mesh.nodes[tri[0]];
            let p1 = mesh.nodes[tri[1]];
            let p2 = mesh.nodes[tri[2]];
            let mut ex0 = p0[0].min(p1[0]).min(p2[0]);
            let mut ex1 = p0[0].max(p1[0]).max(p2[0]);
            let mut ey0 = p0[1].min(p1[1]).min(p2[1]);
            let mut ey1 = p0[1].max(p1[1]).max(p2[1]);
            if let Some(mids_v) = mids {
                // Include quadratic Bezier control points so curved edges are guaranteed inside AABB.
                let m = mids_v[t];
                let pairs = [
                    (p0, p1, mesh.nodes[m[0]]),
                    (p1, p2, mesh.nodes[m[1]]),
                    (p2, p0, mesh.nodes[m[2]]),
                ];
                for (a, b, pm) in pairs {
                    let cx = 2.0 * pm[0] - 0.5 * (a[0] + b[0]);
                    let cy = 2.0 * pm[1] - 0.5 * (a[1] + b[1]);
                    ex0 = ex0.min(pm[0]).min(cx);
                    ex1 = ex1.max(pm[0]).max(cx);
                    ey0 = ey0.min(pm[1]).min(cy);
                    ey1 = ey1.max(pm[1]).max(cy);
                }
            }
            let eps_x = 1e-9 * (ex1 - ex0).max(1e-12);
            let eps_y = 1e-9 * (ey1 - ey0).max(1e-12);
            let ix0 = (((ex0 - eps_x - xmin) * inv_dx).floor() as isize).clamp(0, (nx - 1) as isize)
                as usize;
            let ix1 = (((ex1 + eps_x - xmin) * inv_dx).floor() as isize).clamp(0, (nx - 1) as isize)
                as usize;
            let iy0 = (((ey0 - eps_y - ymin) * inv_dy).floor() as isize).clamp(0, (ny - 1) as isize)
                as usize;
            let iy1 = (((ey1 + eps_y - ymin) * inv_dy).floor() as isize).clamp(0, (ny - 1) as isize)
                as usize;
            for iy in iy0..=iy1 {
                for ix in ix0..=ix1 {
                    cells[iy * nx + ix].push(t);
                }
            }
        }

        Self {
            mesh,
            xmin,
            ymin,
            inv_dx,
            inv_dy,
            nx,
            ny,
            cells,
        }
    }

    /// Locate `(x, y)` in the mesh, returning `Some((elem_idx, xi, eta))` if inside.
    fn locate(&self, x: f64, y: f64) -> Option<(usize, f64, f64)> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let fx = (x - self.xmin) * self.inv_dx;
        let fy = (y - self.ymin) * self.inv_dy;
        if fx < 0.0 || fy < 0.0 || fx >= self.nx as f64 || fy >= self.ny as f64 {
            return None;
        }
        let ix = (fx as usize).min(self.nx - 1);
        let iy = (fy as usize).min(self.ny - 1);
        let candidates = &self.cells[iy * self.nx + ix];
        if candidates.is_empty() {
            return None;
        }

        let mut best: Option<(usize, f64, f64, f64)> = None;
        for &t in candidates {
            if let Some((xi, eta, margin)) = self.try_element(t, x, y) {
                if margin >= 0.0 {
                    return Some((t, xi, eta));
                }
                if best.is_none_or(|(_, _, _, bm)| margin > bm) {
                    best = Some((t, xi, eta, margin));
                }
            }
        }
        best.map(|(t, xi, eta, _)| (t, xi, eta))
    }

    /// Test if `(x, y)` lies in element `t`. Returns `Some((xi, eta, margin))`
    /// where `margin = min(xi, eta, 1 - xi - eta) >= -tol`.
    fn try_element(&self, t: usize, x: f64, y: f64) -> Option<(f64, f64, f64)> {
        let tri = self.mesh.tris[t];
        let p0 = self.mesh.nodes[tri[0]];
        let p1 = self.mesh.nodes[tri[1]];
        let p2 = self.mesh.nodes[tri[2]];
        let dx1 = p1[0] - p0[0];
        let dx2 = p2[0] - p0[0];
        let dy1 = p1[1] - p0[1];
        let dy2 = p2[1] - p0[1];
        let det = dx1 * dy2 - dx2 * dy1;
        if det <= 0.0 || !det.is_finite() {
            return None;
        }
        let rx = x - p0[0];
        let ry = y - p0[1];
        let mut xi = (dy2 * rx - dx2 * ry) / det;
        let mut eta = (-dy1 * rx + dx1 * ry) / det;

        if !self.mesh.quadratic {
            let tol = 1e-11;
            let margin = xi.min(eta).min(1.0 - xi - eta);
            return (margin >= -tol).then_some((xi, eta, margin));
        }

        let mids = self.mesh.mid.as_ref()?;
        let m = mids[t];
        let coords = [
            p0,
            p1,
            p2,
            self.mesh.nodes[m[0]],
            self.mesh.nodes[m[1]],
            self.mesh.nodes[m[2]],
        ];
        // Quick rejection for points far outside the reference triangle.
        if xi < -0.5 || eta < -0.5 || xi + eta > 1.5 {
            return None;
        }
        let h_scale = det.sqrt().max(1e-12);
        // Newton iteration on isoparametric P2 map F(xi, eta) = (x, y).
        for _ in 0..12 {
            let (n, dxi, deta) = p2_shape(xi, eta);
            let mut px = 0.0;
            let mut py = 0.0;
            let mut x_xi = 0.0;
            let mut x_eta = 0.0;
            let mut y_xi = 0.0;
            let mut y_eta = 0.0;
            for a in 0..6 {
                px += n[a] * coords[a][0];
                py += n[a] * coords[a][1];
                x_xi += dxi[a] * coords[a][0];
                x_eta += deta[a] * coords[a][0];
                y_xi += dxi[a] * coords[a][1];
                y_eta += deta[a] * coords[a][1];
            }
            let fx = px - x;
            let fy = py - y;
            if fx.hypot(fy) <= 1e-13 * h_scale {
                break;
            }
            let jdet = x_xi * y_eta - x_eta * y_xi;
            if !jdet.is_finite() || jdet.abs() <= 1e-30 {
                return None;
            }
            let d_xi = (y_eta * fx - x_eta * fy) / jdet;
            let d_eta = (-y_xi * fx + x_xi * fy) / jdet;
            xi -= d_xi;
            eta -= d_eta;
            if !xi.is_finite() || !eta.is_finite() || xi.abs() > 10.0 || eta.abs() > 10.0 {
                return None;
            }
            if d_xi.hypot(d_eta) <= 1e-14 {
                break;
            }
        }
        // Final residual check.
        let (n, _, _) = p2_shape(xi, eta);
        let mut px = 0.0;
        let mut py = 0.0;
        for a in 0..6 {
            px += n[a] * coords[a][0];
            py += n[a] * coords[a][1];
        }
        if (px - x).hypot(py - y) > 1e-9 * h_scale {
            return None;
        }
        let tol = 1e-10;
        let margin = xi.min(eta).min(1.0 - xi - eta);
        (margin >= -tol).then_some((xi, eta, margin))
    }
}

// ---------------------------------------------------------------------------
// Argument parsing for interpolateSolution and evaluateGradient

enum ResultsSource {
    Stationary {
        mesh: Arc<FeMesh>,
        data: Matrix,
        n_comp: usize,
    },
    TimeDependent {
        mesh: Arc<FeMesh>,
        data: Matrix,
        n_comp: usize,
        n_times: usize,
    },
    Eigen {
        mesh: Arc<FeMesh>,
        data: Matrix,
        n_comp: usize,
        n_modes: usize,
    },
}

fn extract_results(e: &Engine, v: &Value, who: &str) -> Result<ResultsSource> {
    if let Some(r) = objects::pde_stationary_results_value(e, v) {
        let n_comp = r.nodal_solution.cols().max(1);
        return Ok(ResultsSource::Stationary {
            mesh: r.mesh.clone(),
            data: r.nodal_solution.clone(),
            n_comp,
        });
    }
    if let Some(r) = objects::pde_time_dependent_results_value(e, v) {
        let dims = r.nodal_solution.dims();
        let (n_comp, n_times) = if dims.len() >= 3 {
            (dims[1], dims[2])
        } else {
            (1, r.nodal_solution.cols())
        };
        return Ok(ResultsSource::TimeDependent {
            mesh: r.mesh.clone(),
            data: r.nodal_solution.clone(),
            n_comp,
            n_times,
        });
    }
    if let Some(r) = objects::pde_eigen_results_value(e, v) {
        // `N x Ncomp x 1` collapses to `N x Ncomp`, so the component count is stored
        // on the results object rather than inferred from Eigenvectors dimensions.
        let n_comp = r.n_comp.max(1);
        let n_modes = r.eigenvectors.cols() / n_comp;
        return Ok(ResultsSource::Eigen {
            mesh: r.mesh.clone(),
            data: r.eigenvectors.clone(),
            n_comp,
            n_modes,
        });
    }
    Err(format!(
        "{who}: first argument must be a pde.StationaryResults, pde.TimeDependentResults, or pde.EigenResults object"
    ))
}

fn query_matrix(v: &Value, label: &str) -> Result<Matrix> {
    let m = match v {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => m.clone(),
        Value::Logical(l) | Value::Container(Array::Logical(l)) => {
            let data = l.data().iter().map(|&b| f64::from(b)).collect();
            Matrix::from_column_vec(l.rows(), l.cols(), data)?
        }
        Value::Container(Array::Typed(a)) => a.to_matrix()?,
        _ => return Err(format!("{label} must be a real numeric array")),
    };
    if m.is_nd() {
        return Err(format!("{label} must be a 2-D numeric array"));
    }
    Ok(m)
}

fn parse_index_list(v: &Value, max_val: usize, label: &str) -> Result<Vec<usize>> {
    let m = super::types::numeric_matrix(v, label)?;
    if m.is_empty() || (m.rows() != 1 && m.cols() != 1) {
        return Err(format!(
            "{label} must be a nonempty vector of positive integer indices"
        ));
    }
    let mut out = Vec::with_capacity(m.len());
    for &x in m.data() {
        if x < 1.0 || x.fract() != 0.0 || x > max_val as f64 {
            return Err(format!(
                "{label}: index {x} out of range (expected integers in 1..{max_val})"
            ));
        }
        out.push(x as usize - 1);
    }
    Ok(out)
}

struct QuerySpec {
    mesh: Arc<FeMesh>,
    data: Matrix,
    n_comp_total: usize,
    is_time_or_eig: bool,
    xq: Vec<f64>,
    yq: Vec<f64>,
    eq_indices: Vec<usize>,
    time_indices: Vec<usize>,
}

fn parse_query_args(e: &Engine, args: &[Value], who: &str) -> Result<QuerySpec> {
    if args.len() < 2 || args.len() > 5 {
        return Err(format!(
            "{who}: expected {who}(results, xq, yq[, iU][, iT]) or {who}(results, querypoints[, iU][, iT])"
        ));
    }
    let src = extract_results(e, &args[0], who)?;
    let (mesh, data, n_comp_total, n_extra, is_time_or_eig) = match src {
        ResultsSource::Stationary { mesh, data, n_comp } => (mesh, data, n_comp, 1usize, false),
        ResultsSource::TimeDependent {
            mesh,
            data,
            n_comp,
            n_times,
        } => (mesh, data, n_comp, n_times, true),
        ResultsSource::Eigen {
            mesh,
            data,
            n_comp,
            n_modes,
        } => (mesh, data, n_comp, n_modes, true),
    };

    // Determine whether query coordinates are passed as (xq, yq) or a single 2xM matrix.
    let (xq, yq, rest) = if args.len() == 2 {
        let qp = query_matrix(&args[1], &format!("{who}: querypoints"))?;
        if qp.rows() != 2 {
            return Err(format!(
                "{who}: querypoints must be a 2-by-M matrix of [x; y] coordinates"
            ));
        }
        let m = qp.cols();
        let mut xq = Vec::with_capacity(m);
        let mut yq = Vec::with_capacity(m);
        for j in 0..m {
            xq.push(qp.get(0, j));
            yq.push(qp.get(1, j));
        }
        (xq, yq, &args[2..])
    } else {
        let a1 = query_matrix(&args[1], &format!("{who}: xq"))?;
        let a2 = query_matrix(&args[2], &format!("{who}: yq"))?;
        if a1.rows() == a2.rows() && a1.cols() == a2.cols() {
            (a1.data().to_vec(), a2.data().to_vec(), &args[3..])
        } else if a1.rows() == 2 && (a2.rows() == 1 || a2.cols() == 1) {
            // (results, querypoints, iU[, iT])
            let m = a1.cols();
            let mut xq = Vec::with_capacity(m);
            let mut yq = Vec::with_capacity(m);
            for j in 0..m {
                xq.push(a1.get(0, j));
                yq.push(a1.get(1, j));
            }
            (xq, yq, &args[2..])
        } else {
            return Err(format!(
                "{who}: xq and yq must have the same dimensions (got {}x{} and {}x{})",
                a1.rows(),
                a1.cols(),
                a2.rows(),
                a2.cols()
            ));
        }
    };

    let (eq_indices, time_indices) = if !is_time_or_eig {
        match rest {
            [] => ((0..n_comp_total).collect(), vec![0]),
            [iu] => (
                parse_index_list(iu, n_comp_total, &format!("{who}: equation index"))?,
                vec![0],
            ),
            _ => {
                return Err(format!(
                    "{who}: too many input arguments for stationary results"
                ));
            }
        }
    } else if n_comp_total == 1 {
        match rest {
            [] => (vec![0], (0..n_extra).collect()),
            [it] => (
                vec![0],
                parse_index_list(it, n_extra, &format!("{who}: time/mode index"))?,
            ),
            [iu, it] => (
                parse_index_list(iu, n_comp_total, &format!("{who}: equation index"))?,
                parse_index_list(it, n_extra, &format!("{who}: time/mode index"))?,
            ),
            _ => return Err(format!("{who}: too many input arguments")),
        }
    } else {
        match rest {
            [] => ((0..n_comp_total).collect(), (0..n_extra).collect()),
            [iu] => (
                parse_index_list(iu, n_comp_total, &format!("{who}: equation index"))?,
                (0..n_extra).collect(),
            ),
            [iu, it] => (
                parse_index_list(iu, n_comp_total, &format!("{who}: equation index"))?,
                parse_index_list(it, n_extra, &format!("{who}: time/mode index"))?,
            ),
            _ => return Err(format!("{who}: too many input arguments")),
        }
    };

    Ok(QuerySpec {
        mesh,
        data,
        n_comp_total,
        is_time_or_eig,
        xq,
        yq,
        eq_indices,
        time_indices,
    })
}

fn shape_output(
    nq: usize,
    n_comp_total: usize,
    is_time_or_eig: bool,
    nu: usize,
    nt: usize,
    buf: Vec<f64>,
) -> Result<Matrix> {
    if !is_time_or_eig {
        Matrix::from_column_vec(nq, nu, buf)
    } else if n_comp_total == 1 {
        Matrix::from_column_vec(nq, nt, buf)
    } else if nt == 1 {
        Matrix::from_column_vec(nq, nu, buf)
    } else {
        Matrix::from_column_vec(nq, nu * nt, buf)?.with_dims(&[nq, nu, nt])
    }
}

/// `interpolateSolution(results, xq, yq[, iU][, iT])`
pub(crate) fn interpolate_solution(e: &Engine, args: &[Value]) -> Result<Value> {
    let spec = parse_query_args(e, args, "interpolateSolution")?;
    let nq = spec.xq.len();
    let nu = spec.eq_indices.len();
    let nt = spec.time_indices.len();
    let n_nodes = spec.mesh.nodes.len();
    let n_comp_total = spec.n_comp_total;
    let sol = spec.data.data();

    let locator = MeshLocator::new(&spec.mesh);
    let mut out = vec![f64::NAN; nq * nu * nt];
    let mids = spec.mesh.mid.as_ref();

    for q in 0..nq {
        if q % 1024 == 0 {
            e.check_cancel()?;
        }
        let Some((t, xi, eta)) = locator.locate(spec.xq[q], spec.yq[q]) else {
            continue;
        };
        if spec.mesh.quadratic {
            let m = mids.unwrap()[t];
            let c = spec.mesh.tris[t];
            let dofs = [c[0], c[1], c[2], m[0], m[1], m[2]];
            let (n, _, _) = p2_shape(xi, eta);
            for (ti_out, &ti) in spec.time_indices.iter().enumerate() {
                for (ui_out, &ui) in spec.eq_indices.iter().enumerate() {
                    let base = (ti * n_comp_total + ui) * n_nodes;
                    let mut val = 0.0;
                    for a in 0..6 {
                        val += n[a] * sol[base + dofs[a]];
                    }
                    out[(ti_out * nu + ui_out) * nq + q] = val;
                }
            }
        } else {
            let dofs = spec.mesh.tris[t];
            let n = p1_shape(xi, eta);
            for (ti_out, &ti) in spec.time_indices.iter().enumerate() {
                for (ui_out, &ui) in spec.eq_indices.iter().enumerate() {
                    let base = (ti * n_comp_total + ui) * n_nodes;
                    let val = n[0] * sol[base + dofs[0]]
                        + n[1] * sol[base + dofs[1]]
                        + n[2] * sol[base + dofs[2]];
                    out[(ti_out * nu + ui_out) * nq + q] = val;
                }
            }
        }
    }

    Ok(Value::Matrix(shape_output(
        nq,
        n_comp_total,
        spec.is_time_or_eig,
        nu,
        nt,
        out,
    )?))
}

/// `[gx, gy] = evaluateGradient(results, xq, yq[, iU][, iT])`
pub(crate) fn evaluate_gradient(e: &Engine, args: &[Value], count: usize) -> Result<Vec<Value>> {
    if count > 2 {
        return Err("evaluateGradient: too many output arguments".into());
    }
    let spec = parse_query_args(e, args, "evaluateGradient")?;
    let nq = spec.xq.len();
    let nu = spec.eq_indices.len();
    let nt = spec.time_indices.len();
    let n_nodes = spec.mesh.nodes.len();
    let n_comp_total = spec.n_comp_total;
    let sol = spec.data.data();

    let locator = MeshLocator::new(&spec.mesh);
    let mut gx_out = vec![f64::NAN; nq * nu * nt];
    let mut gy_out = vec![f64::NAN; nq * nu * nt];
    let mids = spec.mesh.mid.as_ref();

    for q in 0..nq {
        if q % 1024 == 0 {
            e.check_cancel()?;
        }
        let Some((t, xi, eta)) = locator.locate(spec.xq[q], spec.yq[q]) else {
            continue;
        };
        if spec.mesh.quadratic {
            let m = mids.unwrap()[t];
            let c = spec.mesh.tris[t];
            let dofs = [c[0], c[1], c[2], m[0], m[1], m[2]];
            let coords = [
                spec.mesh.nodes[dofs[0]],
                spec.mesh.nodes[dofs[1]],
                spec.mesh.nodes[dofs[2]],
                spec.mesh.nodes[dofs[3]],
                spec.mesh.nodes[dofs[4]],
                spec.mesh.nodes[dofs[5]],
            ];
            let (_, dxi, deta) = p2_shape(xi, eta);
            let (_, dnx, dny) = p2_jacobian_and_grads(&coords, &dxi, &deta)?;
            for (ti_out, &ti) in spec.time_indices.iter().enumerate() {
                for (ui_out, &ui) in spec.eq_indices.iter().enumerate() {
                    let base = (ti * n_comp_total + ui) * n_nodes;
                    let mut gx = 0.0;
                    let mut gy = 0.0;
                    for a in 0..6 {
                        let u_a = sol[base + dofs[a]];
                        gx += dnx[a] * u_a;
                        gy += dny[a] * u_a;
                    }
                    let idx = (ti_out * nu + ui_out) * nq + q;
                    gx_out[idx] = gx;
                    gy_out[idx] = gy;
                }
            }
        } else {
            let dofs = spec.mesh.tris[t];
            let coords = [
                spec.mesh.nodes[dofs[0]],
                spec.mesh.nodes[dofs[1]],
                spec.mesh.nodes[dofs[2]],
            ];
            let (_, dnx, dny) = p1_jacobian_and_grads(&coords)?;
            for (ti_out, &ti) in spec.time_indices.iter().enumerate() {
                for (ui_out, &ui) in spec.eq_indices.iter().enumerate() {
                    let base = (ti * n_comp_total + ui) * n_nodes;
                    let gx = dnx[0] * sol[base + dofs[0]]
                        + dnx[1] * sol[base + dofs[1]]
                        + dnx[2] * sol[base + dofs[2]];
                    let gy = dny[0] * sol[base + dofs[0]]
                        + dny[1] * sol[base + dofs[1]]
                        + dny[2] * sol[base + dofs[2]];
                    let idx = (ti_out * nu + ui_out) * nq + q;
                    gx_out[idx] = gx;
                    gy_out[idx] = gy;
                }
            }
        }
    }

    let gx_mat = shape_output(nq, n_comp_total, spec.is_time_or_eig, nu, nt, gx_out)?;
    if count == 0 {
        return Ok(vec![]);
    }
    if count == 1 {
        return Ok(vec![Value::Matrix(gx_mat)]);
    }
    let gy_mat = shape_output(nq, n_comp_total, spec.is_time_or_eig, nu, nt, gy_out)?;
    Ok(vec![Value::Matrix(gx_mat), Value::Matrix(gy_mat)])
}
