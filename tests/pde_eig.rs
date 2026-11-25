//! Tests for 2-D PDE eigenvalue solver (`solvepdeeig`) and `pde.EigenResults` (Batch 8b3).
// pde_solve.rs loads the same helper; each test module keeps its own copy.
#[allow(clippy::duplicate_mod)]
#[path = "support/pde_disk_fixture.rs"]
mod disk_fixture;

use disk_fixture::{UNIT_DISK, disk_model};
use matrust::{Engine, Matrix};
use std::f64::consts::PI;
use std::path::PathBuf;

const UNIT_SQUARE: &str = "\
R1 = [3; 4; 0; 1; 1; 0; 0; 0; 1; 1];\n\
dl = decsg(R1);\n";

fn run(src: &str) -> Engine {
    let mut e = Engine::default();
    if let Err(err) = e.execute(src) {
        panic!("{src}\n{err}");
    }
    e
}

fn mat(e: &Engine, name: &str) -> Matrix {
    e.variables[name].clone()
}

fn scalar(e: &Engine, name: &str) -> f64 {
    e.variables[name].as_scalar().unwrap()
}

fn error(src: &str) -> String {
    Engine::default().execute(src).unwrap_err()
}

/// Independent FEM stiffness `K` (c=1, a=0) and mass `M` (d=1) matvecs on a linear
/// or quadratic triangular mesh, used to verify the generalized eigenpair residual
/// `||K v - lambda M v||_2 / (||K v||_2 + |lambda| ||M v||_2)` on interior (free) DOFs
/// and `M`-orthonormality `V^T M V = I`.
fn fem_km_apply(
    nodes: &Matrix,
    elems: &Matrix,
    v: &[f64],
    c_val: f64,
    d_val: f64,
) -> (Vec<f64>, Vec<f64>) {
    let n_nodes = nodes.cols();
    let n_elems = elems.cols();
    let is_quad = elems.rows() == 6;
    let mut kv = vec![0.0; n_nodes];
    let mut mv = vec![0.0; n_nodes];

    // 7-point degree-5 Dunavant quadrature on reference triangle (area = 1/2).
    let s15 = 15.0_f64.sqrt();
    let r1 = (6.0 - s15) / 21.0;
    let s1 = (9.0 + 2.0 * s15) / 21.0;
    let w1 = (155.0 - s15) / 2400.0;
    let r2 = (6.0 + s15) / 21.0;
    let s2 = (9.0 - 2.0 * s15) / 21.0;
    let w2 = (155.0 + s15) / 2400.0;
    let qpts = [
        (1.0 / 3.0, 1.0 / 3.0, 9.0 / 80.0),
        (r1, r1, w1),
        (s1, r1, w1),
        (r1, s1, w1),
        (r2, r2, w2),
        (s2, r2, w2),
        (r2, s2, w2),
    ];

    for t in 0..n_elems {
        if !is_quad {
            let idxs = [
                elems.get(0, t) as usize - 1,
                elems.get(1, t) as usize - 1,
                elems.get(2, t) as usize - 1,
            ];
            let p: [[f64; 2]; 3] =
                std::array::from_fn(|a| [nodes.get(0, idxs[a]), nodes.get(1, idxs[a])]);
            let det = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
                - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
            let dnx = [
                (p[1][1] - p[2][1]) / det,
                (p[2][1] - p[0][1]) / det,
                (p[0][1] - p[1][1]) / det,
            ];
            let dny = [
                (p[2][0] - p[1][0]) / det,
                (p[0][0] - p[2][0]) / det,
                (p[1][0] - p[0][0]) / det,
            ];
            let ve: [f64; 3] = std::array::from_fn(|a| v[idxs[a]]);
            let gx = dnx[0] * ve[0] + dnx[1] * ve[1] + dnx[2] * ve[2];
            let gy = dny[0] * ve[0] + dny[1] * ve[1] + dny[2] * ve[2];
            let area = 0.5 * det;
            for i in 0..3 {
                kv[idxs[i]] += area * c_val * (dnx[i] * gx + dny[i] * gy);
            }
            for &(xi, eta, w) in &qpts {
                let n = [1.0 - xi - eta, xi, eta];
                let uh = n[0] * ve[0] + n[1] * ve[1] + n[2] * ve[2];
                let wd = w * det * d_val * uh;
                for i in 0..3 {
                    mv[idxs[i]] += wd * n[i];
                }
            }
        } else {
            let idxs: [usize; 6] = std::array::from_fn(|a| elems.get(a, t) as usize - 1);
            let p: [[f64; 2]; 6] =
                std::array::from_fn(|a| [nodes.get(0, idxs[a]), nodes.get(1, idxs[a])]);
            let ve: [f64; 6] = std::array::from_fn(|a| v[idxs[a]]);
            for &(xi, eta, w) in &qpts {
                let l0 = 1.0 - xi - eta;
                let l1 = xi;
                let l2 = eta;
                let n = [
                    l0 * (2.0 * l0 - 1.0),
                    l1 * (2.0 * l1 - 1.0),
                    l2 * (2.0 * l2 - 1.0),
                    4.0 * l0 * l1,
                    4.0 * l1 * l2,
                    4.0 * l2 * l0,
                ];
                let dxi = [
                    -(4.0 * l0 - 1.0),
                    4.0 * l1 - 1.0,
                    0.0,
                    4.0 * (l0 - l1),
                    4.0 * l2,
                    -4.0 * l2,
                ];
                let deta = [
                    -(4.0 * l0 - 1.0),
                    0.0,
                    4.0 * l2 - 1.0,
                    -4.0 * l1,
                    4.0 * l1,
                    4.0 * (l0 - l2),
                ];
                let (mut x_xi, mut x_eta, mut y_xi, mut y_eta) = (0.0, 0.0, 0.0, 0.0);
                for a in 0..6 {
                    x_xi += dxi[a] * p[a][0];
                    x_eta += deta[a] * p[a][0];
                    y_xi += dxi[a] * p[a][1];
                    y_eta += deta[a] * p[a][1];
                }
                let det = x_xi * y_eta - x_eta * y_xi;
                let mut dnx = [0.0; 6];
                let mut dny = [0.0; 6];
                let (mut gx, mut gy, mut uh) = (0.0, 0.0, 0.0);
                for a in 0..6 {
                    dnx[a] = (y_eta * dxi[a] - y_xi * deta[a]) / det;
                    dny[a] = (-x_eta * dxi[a] + x_xi * deta[a]) / det;
                    gx += dnx[a] * ve[a];
                    gy += dny[a] * ve[a];
                    uh += n[a] * ve[a];
                }
                let wd = w * det;
                for i in 0..6 {
                    kv[idxs[i]] += wd * c_val * (dnx[i] * gx + dny[i] * gy);
                    mv[idxs[i]] += wd * d_val * uh * n[i];
                }
            }
        }
    }
    (kv, mv)
}

#[test]
#[allow(clippy::needless_range_loop)]
fn unit_square_dirichlet_quadratic_eigenvalues_and_residuals() {
    // Unit square [0,1]^2, -Delta u = lambda u, u = 0 on all edges.
    // Exact Dirichlet Laplacian eigenvalues: (m^2 + n^2) * pi^2 for integers m, n >= 1:
    //   2*pi^2, 5*pi^2 (multiplicity 2), 8*pi^2, 10*pi^2 (multiplicity 2), ...
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.05, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         res = solvepdeeig(m, [0, 90]);\n\
         lam = res.Eigenvalues;\n\
         V = res.Eigenvectors;\n\
         P = res.Mesh.Nodes;\n\
         E = res.Mesh.Elements;\n\
         bd = findNodes(res.Mesh, 'region', 'Edge', 1:4);\n"
    ));

    let lam = mat(&e, "lam");
    let v_mat = mat(&e, "V");
    let p = mat(&e, "P");
    let elems = mat(&e, "E");
    let bd = mat(&e, "bd");

    let pi2 = PI * PI;
    let expected = [2.0 * pi2, 5.0 * pi2, 5.0 * pi2, 8.0 * pi2];
    assert_eq!(lam.cols(), 1, "Eigenvalues must be k x 1");
    assert_eq!(
        lam.rows(),
        expected.len(),
        "expected 4 eigenvalues in [0, 90], got {}: {:?}",
        lam.rows(),
        lam.data()
    );
    assert_eq!(v_mat.rows(), p.cols());
    assert_eq!(v_mat.cols(), expected.len());

    // Ascending order and relative accuracy <= 1e-3, including multiplicity 2 at 5*pi^2.
    for i in 1..lam.rows() {
        assert!(
            lam.get(i, 0) >= lam.get(i - 1, 0),
            "eigenvalues not ascending"
        );
    }
    for (i, &ex) in expected.iter().enumerate() {
        let got = lam.get(i, 0);
        let rel = (got - ex).abs() / ex;
        assert!(
            rel <= 1e-3,
            "square mode {i}: got {got:.6}, expected {ex:.6} (rel {rel:.3e} > 1e-3)"
        );
    }
    let mult2_gap = (lam.get(2, 0) - lam.get(1, 0)).abs() / (5.0 * pi2);
    assert!(
        mult2_gap <= 1e-3,
        "multiplicity-2 pair at 5*pi^2 split by {mult2_gap:.3e}"
    );

    // Check Dirichlet boundary nodes are exactly 0, residual <= 1e-8, and M-orthonormality.
    let n_nodes = p.cols();
    let mut is_bd = vec![false; n_nodes];
    for &id in bd.data() {
        is_bd[id as usize - 1] = true;
    }

    let mut m_cols: Vec<Vec<f64>> = Vec::new();
    for j in 0..lam.rows() {
        let vj: Vec<f64> = (0..n_nodes).map(|r| v_mat.get(r, j)).collect();
        for (i, &b) in is_bd.iter().enumerate() {
            if b {
                assert_eq!(vj[i], 0.0, "mode {j}: Dirichlet node {i} must be 0");
            }
        }
        let (kv, mv) = fem_km_apply(&p, &elems, &vj, 1.0, 1.0);
        let lj = lam.get(j, 0);
        let mut res_sq = 0.0;
        let mut kv_sq = 0.0;
        let mut mv_sq = 0.0;
        for i in 0..n_nodes {
            if !is_bd[i] {
                let d = kv[i] - lj * mv[i];
                res_sq += d * d;
                kv_sq += kv[i] * kv[i];
                mv_sq += mv[i] * mv[i];
            }
        }
        let rel_res = res_sq.sqrt() / (kv_sq.sqrt() + lj.abs() * mv_sq.sqrt());
        assert!(
            rel_res <= 1e-8,
            "square mode {j}: residual {rel_res:.3e} > 1e-8"
        );
        m_cols.push(mv);
    }

    // M-orthonormality: V^T M V = I.
    for i in 0..lam.rows() {
        for j in 0..lam.rows() {
            let mut g = 0.0;
            for r in 0..n_nodes {
                g += v_mat.get(r, i) * m_cols[j][r];
            }
            let target = if i == j { 1.0 } else { 0.0 };
            assert!(
                (g - target).abs() <= 1e-8,
                "V^T M V ({i},{j}) = {g:.6e}, expected {target}"
            );
        }
    }
}

#[test]
fn unit_disk_dirichlet_bessel_zeros() {
    // Unit disk r <= 1, -Delta u = lambda u, u = 0 on boundary.
    // Exact eigenvalues are squares of zeros of Bessel functions J_n(r):
    // Source: Abramowitz & Stegun, Table 9.5, and scipy.special.jn_zeros(n, k)^2:
    //   j_{0,1}^2 = 5.7831859629467845 (multiplicity 1)
    //   j_{1,1}^2 = 14.681970642123893 (multiplicity 2)
    //   j_{2,1}^2 = 26.374616427163390 (multiplicity 2)
    //   j_{0,2}^2 = 30.471262343662086 (multiplicity 1)
    let e = run(&format!(
        "{UNIT_DISK}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.1, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         res = solvepdeeig(m, [0, 35]);\n\
         lam = res.Eigenvalues;\n"
    ));

    let lam = mat(&e, "lam");
    let expected = [
        5.783_185_962_946_784,
        14.681_970_642_123_893,
        14.681_970_642_123_893,
        26.374_616_427_163_39,
        26.374_616_427_163_39,
        30.471_262_343_662_086,
    ];
    assert_eq!(
        lam.rows(),
        expected.len(),
        "expected 6 disk eigenvalues in [0, 35], got {}: {:?}",
        lam.rows(),
        lam.data()
    );
    for (i, &ex) in expected.iter().enumerate() {
        let got = lam.get(i, 0);
        let rel = (got - ex).abs() / ex;
        assert!(
            rel <= 1e-3,
            "disk mode {i}: got {got:.6}, expected {ex:.6} (rel {rel:.3e} > 1e-3)"
        );
    }
}

fn assert_matches_eigen_fixture(name: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("pde")
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read fixture {}: {err}", path.display()));
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let fx_nodes: Vec<Vec<f64>> = serde_json::from_value(v["nodes"].clone()).unwrap();
    let fx_elems: Vec<Vec<usize>> = serde_json::from_value(v["elements"].clone()).unwrap();
    let fx_range: Vec<f64> = serde_json::from_value(v["range"].clone()).unwrap();
    let fx_lambda: Vec<f64> = serde_json::from_value(v["lambda"].clone()).unwrap();

    let mut e = disk_model(&fx_nodes, &fx_elems);
    e.execute(&format!(
        "specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         res = solvepdeeig(m, [{}, {}]);\n\
         P = res.Mesh.Nodes; E = res.Mesh.Elements; lam = res.Eigenvalues;\n",
        fx_range[0], fx_range[1]
    ))
    .unwrap();

    let p = mat(&e, "P");
    let elems = mat(&e, "E");
    let lam = mat(&e, "lam");

    assert_eq!(p.rows(), fx_nodes.len(), "{name}: Nodes rows");
    assert_eq!(p.cols(), fx_nodes[0].len(), "{name}: Nodes cols");
    assert_eq!(elems.rows(), fx_elems.len(), "{name}: Elements rows");
    assert_eq!(elems.cols(), fx_elems[0].len(), "{name}: Elements cols");
    assert_eq!(
        lam.rows(),
        fx_lambda.len(),
        "{name}: eigenvalue count mismatch (got {}, expected {})",
        lam.rows(),
        fx_lambda.len()
    );
    for (i, &ref_l) in fx_lambda.iter().enumerate() {
        let got = lam.get(i, 0);
        let rel = (got - ref_l).abs() / ref_l.abs().max(1.0);
        assert!(
            rel <= 1e-8,
            "{name} mode {i}: got {got:.12}, skfem {ref_l:.12} (rel {rel:.3e} > 1e-8)"
        );
    }
}

#[test]
fn fixture_disk_eigen_linear_matches_skfem() {
    assert_matches_eigen_fixture("disk_eigen_linear");
}

#[test]
fn fixture_disk_eigen_quadratic_matches_skfem() {
    assert_matches_eigen_fixture("disk_eigen_quadratic");
}

#[test]
fn range_filtering_neg_inf_and_empty_range() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.1, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         n_nodes = size(m.Mesh.Nodes, 2);\n\
         r_all = solvepdeeig(m, [-Inf, 90]);\n\
         r_mid = solvepdeeig(m, [30, 70]);\n\
         r_empty = solvepdeeig(m, [25, 35]);\n\
         r_neg = solvepdeeig(m, [-Inf, 0]);\n\
         lam_all = r_all.Eigenvalues;\n\
         lam_mid = r_mid.Eigenvalues;\n\
         lam_empty = r_empty.Eigenvalues;\n\
         vec_empty = r_empty.Eigenvectors;\n\
         lam_neg = r_neg.Eigenvalues;\n\
         vec_neg = r_neg.Eigenvectors;\n"
    ));

    let n_nodes = scalar(&e, "n_nodes") as usize;
    let lam_all = mat(&e, "lam_all");
    let lam_mid = mat(&e, "lam_mid");
    let lam_empty = mat(&e, "lam_empty");
    let vec_empty = mat(&e, "vec_empty");
    let lam_neg = mat(&e, "lam_neg");
    let vec_neg = mat(&e, "vec_neg");

    // [-Inf, 90] has 4 eigenvalues: 2*pi^2 (~19.74), 5*pi^2 (~49.35) x2, 8*pi^2 (~78.96).
    assert_eq!(lam_all.rows(), 4);
    assert_eq!(lam_all.cols(), 1);

    // [30, 70] has only the multiplicity-2 pair at 5*pi^2.
    assert_eq!(lam_mid.rows(), 2);
    for &l in lam_mid.data() {
        assert!((30.0..=70.0).contains(&l), "out-of-range eigenvalue {l}");
    }

    // [25, 35] has no eigenvalues -> 0x1 and Nx0.
    assert_eq!(lam_empty.rows(), 0);
    assert_eq!(lam_empty.cols(), 1);
    assert_eq!(vec_empty.rows(), n_nodes);
    assert_eq!(vec_empty.cols(), 0);

    // [-Inf, 0] has no eigenvalues -> 0x1 and Nx0.
    assert_eq!(lam_neg.rows(), 0);
    assert_eq!(lam_neg.cols(), 1);
    assert_eq!(vec_neg.rows(), n_nodes);
    assert_eq!(vec_neg.cols(), 0);
}

#[test]
fn eigen_results_object_properties_and_methods() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.2, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         r = solvepdeeig(m, [0, 60]);\n\
         is_eig = double(isa(r, 'pde.EigenResults'));\n\
         is_stat = double(isa(r, 'pde.StationaryResults'));\n\
         s = get(r);\n\
         l_get = get(r, 'eigenvalues');\n\
         l_prop = r.Eigenvalues;\n\
         eq_get = double(isequal(l_get, l_prop));\n\
         same_mesh = double(isequal(r.Mesh, m.Mesh));\n\
         p = properties(r);\n\
         f = fieldnames(s);\n\
         order_ok = double(strcmp(p{{1}}, 'Eigenvectors') && strcmp(p{{2}}, 'Eigenvalues') && strcmp(p{{3}}, 'Mesh') && strcmp(f{{1}}, 'Eigenvectors') && strcmp(f{{2}}, 'Eigenvalues') && strcmp(f{{3}}, 'Mesh'));\n\
         d = evalc('disp(r)');\n\
         i_vec = strfind(d, 'Eigenvectors:');\n\
         i_val = strfind(d, 'Eigenvalues:');\n\
         i_mesh = strfind(d, 'Mesh:');\n\
         disp_ok = double(numel(i_vec)==1 && numel(i_val)==1 && numel(i_mesh)==1 && i_vec<i_val && i_val<i_mesh);\n"
    ));

    assert_eq!(scalar(&e, "is_eig"), 1.0);
    assert_eq!(scalar(&e, "is_stat"), 0.0);
    assert_eq!(scalar(&e, "eq_get"), 1.0);
    assert_eq!(scalar(&e, "same_mesh"), 1.0);
    assert_eq!(
        scalar(&e, "order_ok"),
        1.0,
        "property order must be Eigenvectors, Eigenvalues, Mesh"
    );
    assert_eq!(
        scalar(&e, "disp_ok"),
        1.0,
        "display order must be Eigenvectors, Eigenvalues, Mesh"
    );

    let err = error(&format!(
        "{UNIT_SQUARE}\
         m = createpde(); geometryFromEdges(m, dl); generateMesh(m, 'Hmax', 0.25);\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         r = solvepdeeig(m, [0, 60]); r.Eigenvalues = 0;\n"
    ));
    assert!(
        err.contains("read-only"),
        "expected read-only error, got: {err}"
    );
}

#[test]
fn unit_square_pure_neumann_includes_constant_mode() {
    // Unit square, no boundary conditions: homogeneous Neumann, -Delta u = lambda u.
    // Spectrum (m^2 + n^2) * pi^2 for integers m, n >= 0, including the constant mode λ = 0.
    // Hmax 0.05 quadratic has n_free > 250, so this exercises the sparse residual scale.
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.05, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         res = solvepdeeig(m, [-1, 50]);\n\
         lam = res.Eigenvalues;\n\
         res0 = solvepdeeig(m, [-Inf, 5]);\n\
         lam0 = res0.Eigenvalues;\n"
    ));

    let lam = mat(&e, "lam");
    let lam0 = mat(&e, "lam0");
    let pi2 = PI * PI;
    let expected = [
        pi2,
        pi2,
        2.0 * pi2,
        4.0 * pi2,
        4.0 * pi2,
        5.0 * pi2,
        5.0 * pi2,
    ];

    assert_eq!(lam.cols(), 1);
    assert_eq!(
        lam.rows(),
        expected.len() + 1,
        "expected 0 then 7 positive eigenvalues in [-1, 50], got {:?}",
        lam.data()
    );
    assert!(
        lam.get(0, 0).abs() <= 1e-8,
        "constant mode: got {}, expected 0",
        lam.get(0, 0)
    );
    for i in 1..lam.rows() {
        assert!(
            lam.get(i, 0) >= lam.get(i - 1, 0),
            "eigenvalues not ascending"
        );
    }
    for (i, &ex) in expected.iter().enumerate() {
        let got = lam.get(i + 1, 0);
        let rel = (got - ex).abs() / ex;
        assert!(
            rel <= 1e-3,
            "Neumann mode {}: got {got:.6}, expected {ex:.6} (rel {rel:.3e} > 1e-3)",
            i + 1
        );
    }

    // [-Inf, 5] contains only the constant mode (next analytic eigenvalue is pi^2).
    assert_eq!(
        lam0.rows(),
        1,
        "expected one eigenvalue in [-Inf, 5], got {:?}",
        lam0.data()
    );
    assert_eq!(lam0.cols(), 1);
    assert!(
        lam0.get(0, 0).abs() <= 1e-8,
        "eigenvalue in [-Inf, 5]: got {}, expected ~0",
        lam0.get(0, 0)
    );
}

#[test]
fn generalized_neumann_and_variable_coefficients() {
    // Unit square with u=0 on x=0,1 (edges 2,4) and du/dn=0 on y=0,1 (edges 1,3),
    // shifted by a = 3 and scaled by d = 2:
    //   -Delta u + 3 u = lambda * 2 u  =>  lambda = ((m*pi)^2 + (n*pi)^2 + 3) / 2
    // for m >= 1, n >= 0. Lowest: (pi^2 + 3)/2 = 6.434802, (2*pi^2 + 3)/2 = 11.369604.
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.1, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(m, 'm', 0, 'd', @(loc,st) 2*ones(1,numel(loc.x)), 'c', 1, 'a', 3, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', [2 4], 'u', 0);\n\
         res = solvepdeeig(m, [0, 15]);\n\
         lam = res.Eigenvalues;\n"
    ));
    let lam = mat(&e, "lam");
    let ex0 = (PI * PI + 3.0) / 2.0;
    let ex1 = (2.0 * PI * PI + 3.0) / 2.0;
    assert_eq!(lam.rows(), 2, "got {:?}", lam.data());
    assert!((lam.get(0, 0) - ex0).abs() / ex0 <= 1e-3);
    assert!((lam.get(1, 0) - ex1).abs() / ex1 <= 1e-3);
}

#[test]
fn error_handling_for_missing_range_and_bad_coefficients() {
    let base = format!(
        "{UNIT_SQUARE}\
         m = createpde(); geometryFromEdges(m, dl); generateMesh(m, 'Hmax', 0.25);\n"
    );

    // Missing range.
    let e1 = error(&format!(
        "{base}specifyCoefficients(m,'m',0,'d',1,'c',1,'a',0,'f',0); solvepdeeig(m);"
    ));
    assert!(e1.contains("Not enough input arguments"), "got: {e1}");

    // Bad range: lo > hi.
    let e2 = error(&format!(
        "{base}specifyCoefficients(m,'m',0,'d',1,'c',1,'a',0,'f',0); solvepdeeig(m, [50, 10]);"
    ));
    assert!(e2.contains("lo <= hi"), "got: {e2}");

    // Bad range: upper bound Inf.
    let e3 = error(&format!(
        "{base}specifyCoefficients(m,'m',0,'d',1,'c',1,'a',0,'f',0); solvepdeeig(m, [0, Inf]);"
    ));
    assert!(e3.contains("finite"), "got: {e3}");

    // Bad range: NaN.
    let e4 = error(&format!(
        "{base}specifyCoefficients(m,'m',0,'d',1,'c',1,'a',0,'f',0); solvepdeeig(m, [NaN, 10]);"
    ));
    assert!(e4.contains("NaN"), "got: {e4}");

    // Nonzero m.
    let e5 = error(&format!(
        "{base}specifyCoefficients(m,'m',1,'d',1,'c',1,'a',0,'f',0); solvepdeeig(m, [0, 50]);"
    ));
    assert!(e5.contains("'m'"), "got: {e5}");

    // Zero d.
    let e6 = error(&format!(
        "{base}specifyCoefficients(m,'m',0,'d',0,'c',1,'a',0,'f',0); solvepdeeig(m, [0, 50]);"
    ));
    assert!(e6.contains("'d'"), "got: {e6}");

    // Negative d.
    let e7 = error(&format!(
        "{base}specifyCoefficients(m,'m',0,'d',-1,'c',1,'a',0,'f',0); solvepdeeig(m, [0, 50]);"
    ));
    assert!(e7.contains("'d'"), "got: {e7}");

    // Missing coefficients.
    let e8 = error(&format!("{base}solvepdeeig(m, [0, 50]);"));
    assert!(e8.contains("no equation coefficients"), "got: {e8}");

    // Nonsymmetric c.
    let e9 = error(&format!(
        "{base}specifyCoefficients(m,'m',0,'d',1,'c',[1;2;3;4],'a',0,'f',0); solvepdeeig(m, [0, 50]);"
    ));
    assert!(e9.contains("Nonsymmetric"), "got: {e9}");
}
