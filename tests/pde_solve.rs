//! Tests for specifyCoefficients, applyBoundaryCondition, solvepde, and
//! pde.StationaryResults (Batch 8b1).
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

fn load_fixture(name: &str) -> (Vec<Vec<f64>>, Vec<Vec<usize>>, Vec<f64>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("pde")
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read fixture {}: {err}", path.display()));
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let nodes: Vec<Vec<f64>> = serde_json::from_value(v["nodes"].clone()).unwrap();
    let elements: Vec<Vec<usize>> = serde_json::from_value(v["elements"].clone()).unwrap();
    let u: Vec<f64> = serde_json::from_value(v["u"].clone()).unwrap();
    (nodes, elements, u)
}

#[allow(clippy::needless_range_loop)]
fn assert_matches_fixture(name: &str, solve: impl FnOnce(&[Vec<f64>], &[Vec<usize>]) -> Engine) {
    let (fx_nodes, fx_elems, fx_u) = load_fixture(name);
    let e = solve(&fx_nodes, &fx_elems);
    let p = mat(&e, "P");
    let elems = mat(&e, "E");
    let u = mat(&e, "u");

    assert_eq!(p.rows(), fx_nodes.len(), "{name}: Nodes row count mismatch");
    assert_eq!(
        p.cols(),
        fx_nodes[0].len(),
        "{name}: Nodes col count mismatch"
    );
    let mut max_node_diff = 0.0f64;
    for r in 0..p.rows() {
        for c in 0..p.cols() {
            max_node_diff = max_node_diff.max((p.get(r, c) - fx_nodes[r][c]).abs());
        }
    }
    assert!(
        max_node_diff <= 1e-12,
        "{name}: mesh Nodes differ from fixture by {max_node_diff:.3e} (> 1e-12)"
    );

    assert_eq!(
        elems.rows(),
        fx_elems.len(),
        "{name}: Elements row count mismatch"
    );
    assert_eq!(
        elems.cols(),
        fx_elems[0].len(),
        "{name}: Elements col count mismatch"
    );
    let mut max_elem_diff = 0.0f64;
    for r in 0..elems.rows() {
        for c in 0..elems.cols() {
            max_elem_diff = max_elem_diff.max((elems.get(r, c) - fx_elems[r][c] as f64).abs());
        }
    }
    assert!(
        max_elem_diff <= 1e-12,
        "{name}: mesh Elements differ from fixture by {max_elem_diff:.3e} (> 1e-12)"
    );

    assert_eq!(
        u.rows(),
        fx_u.len(),
        "{name}: NodalSolution length mismatch"
    );
    assert_eq!(u.cols(), 1, "{name}: NodalSolution must be N x 1");
    let mut max_u_diff = 0.0f64;
    for (i, &u_ref) in fx_u.iter().enumerate() {
        let diff = (u.get(i, 0) - u_ref).abs();
        assert!(
            diff.is_finite(),
            "{name}: nonfinite difference at node {}",
            i + 1
        );
        max_u_diff = max_u_diff.max(diff);
    }
    eprintln!(
        "fixture {name}: max NodalSolution diff {max_u_diff:.3e}, node diff {max_node_diff:.3e}"
    );
    assert!(
        max_u_diff <= 1e-8,
        "{name}: NodalSolution differs from scikit-fem fixture by {max_u_diff:.3e} (> 1e-8)"
    );
}

fn assert_disk_poisson_fixture(name: &str) {
    assert_matches_fixture(name, |nodes, elements| {
        let mut e = disk_model(nodes, elements);
        e.execute(
            "specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 1);\n\
             applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
             res = solvepde(m);\n\
             P = res.Mesh.Nodes; E = res.Mesh.Elements; u = res.NodalSolution;\n",
        )
        .unwrap();
        e
    });
}

#[test]
fn fixture_disk_poisson_linear() {
    assert_disk_poisson_fixture("disk_poisson_linear");
}

#[test]
fn fixture_disk_poisson_quadratic() {
    assert_disk_poisson_fixture("disk_poisson_quadratic");
}

fn assert_square_mixed_fixture(name: &str, order: &str) {
    assert_matches_fixture(name, |_, _| {
        run(&format!(
            "{UNIT_SQUARE}\
             m = createpde();\n\
             geometryFromEdges(m, dl);\n\
             generateMesh(m, 'Hmax', 0.15, 'GeometricOrder', '{order}');\n\
             specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1.5, 'a', 2, 'f', 3);\n\
             applyBoundaryCondition(m, 'dirichlet', 'Edge', [1 2], 'u', 1);\n\
             applyBoundaryCondition(m, 'neumann', 'Edge', [3 4], 'g', 1, 'q', 0.5);\n\
             res = solvepde(m);\n\
             P = res.Mesh.Nodes; E = res.Mesh.Elements; u = res.NodalSolution;\n"
        ))
    });
}

#[test]
fn fixture_square_mixed_linear() {
    assert_square_mixed_fixture("square_mixed_linear", "linear");
}

#[test]
fn fixture_square_mixed_quadratic() {
    assert_square_mixed_fixture("square_mixed_quadratic", "quadratic");
}

/// Compute L2 error ||u_h - u_exact||_{L2} and mean triangle edge length `h`
/// from MATLAB `Nodes` (2xN), `Elements` (3xM or 6xM, 1-based), and `NodalSolution` (Nx1).
fn l2_error_and_h(
    nodes: &Matrix,
    elems: &Matrix,
    u_sol: &Matrix,
    u_exact: impl Fn(f64, f64) -> f64,
) -> (f64, f64) {
    assert_eq!(nodes.rows(), 2);
    let n_elems = elems.cols();
    let is_quad = elems.rows() == 6;

    // Mean edge length over all corner triangle edges (3*M edges).
    let mut edge_sum = 0.0;
    let mut edge_cnt = 0usize;
    for t in 0..n_elems {
        let i0 = elems.get(0, t) as usize - 1;
        let i1 = elems.get(1, t) as usize - 1;
        let i2 = elems.get(2, t) as usize - 1;
        let p0 = [nodes.get(0, i0), nodes.get(1, i0)];
        let p1 = [nodes.get(0, i1), nodes.get(1, i1)];
        let p2 = [nodes.get(0, i2), nodes.get(1, i2)];
        edge_sum += (p1[0] - p0[0]).hypot(p1[1] - p0[1]);
        edge_sum += (p2[0] - p1[0]).hypot(p2[1] - p1[1]);
        edge_sum += (p0[0] - p2[0]).hypot(p0[1] - p2[1]);
        edge_cnt += 3;
    }
    let h = edge_sum / edge_cnt as f64;

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

    let mut err_sq = 0.0;
    for t in 0..n_elems {
        if !is_quad {
            let idxs = [
                elems.get(0, t) as usize - 1,
                elems.get(1, t) as usize - 1,
                elems.get(2, t) as usize - 1,
            ];
            let p: [[f64; 2]; 3] =
                std::array::from_fn(|a| [nodes.get(0, idxs[a]), nodes.get(1, idxs[a])]);
            let u: [f64; 3] = std::array::from_fn(|a| u_sol.get(idxs[a], 0));
            let det = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
                - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
            for &(xi, eta, w) in &qpts {
                let n = [1.0 - xi - eta, xi, eta];
                let x = n[0] * p[0][0] + n[1] * p[1][0] + n[2] * p[2][0];
                let y = n[0] * p[0][1] + n[1] * p[1][1] + n[2] * p[2][1];
                let uh = n[0] * u[0] + n[1] * u[1] + n[2] * u[2];
                let diff = uh - u_exact(x, y);
                err_sq += w * det * diff * diff;
            }
        } else {
            let idxs: [usize; 6] = std::array::from_fn(|a| elems.get(a, t) as usize - 1);
            let p: [[f64; 2]; 6] =
                std::array::from_fn(|a| [nodes.get(0, idxs[a]), nodes.get(1, idxs[a])]);
            let u: [f64; 6] = std::array::from_fn(|a| u_sol.get(idxs[a], 0));
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
                let mut x = 0.0;
                let mut y = 0.0;
                let mut uh = 0.0;
                let (mut x_xi, mut x_eta, mut y_xi, mut y_eta) = (0.0, 0.0, 0.0, 0.0);
                for a in 0..6 {
                    x += n[a] * p[a][0];
                    y += n[a] * p[a][1];
                    uh += n[a] * u[a];
                    x_xi += dxi[a] * p[a][0];
                    x_eta += deta[a] * p[a][0];
                    y_xi += dxi[a] * p[a][1];
                    y_eta += deta[a] * p[a][1];
                }
                let det = x_xi * y_eta - x_eta * y_xi;
                let diff = uh - u_exact(x, y);
                err_sq += w * det * diff * diff;
            }
        }
    }
    (err_sq.sqrt(), h)
}

fn solve_sine_square(hmax: f64, order: &str) -> (f64, f64) {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', {hmax}, 'GeometricOrder', '{order}');\n\
         f_rhs = @(loc, state) 2 * pi^2 * sin(pi * loc.x) .* sin(pi * loc.y);\n\
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', f_rhs);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         res = solvepde(m);\n\
         P = res.Mesh.Nodes;\n\
         E = res.Mesh.Elements;\n\
         u = res.NodalSolution;\n"
    ));
    l2_error_and_h(&mat(&e, "P"), &mat(&e, "E"), &mat(&e, "u"), |x, y| {
        (PI * x).sin() * (PI * y).sin()
    })
}

#[test]
fn disk_poisson_analytic_linear() {
    let e = run(&format!(
        "{UNIT_DISK}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.1, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 1);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         res = solvepde(m);\n\
         P = res.Mesh.Nodes;\n\
         u = res.NodalSolution;\n\
         u_ex = (1 - (P(1,:).^2 + P(2,:).^2))' / 4;\n\
         max_err = max(abs(u - u_ex));\n\
         is_res = double(isa(res, 'pde.StationaryResults'));\n\
         nr = size(u, 1); nc = size(u, 2); nn = size(P, 2);\n"
    ));
    assert_eq!(scalar(&e, "is_res"), 1.0);
    assert_eq!(scalar(&e, "nc"), 1.0);
    assert_eq!(scalar(&e, "nr"), scalar(&e, "nn"));
    let max_err = scalar(&e, "max_err");
    eprintln!("disk Poisson P1 max nodal error {max_err:.3e}");
    assert!(
        max_err <= 1e-3,
        "disk Poisson P1 max nodal error {max_err:.6e} > 1e-3"
    );
}

#[test]
fn l2_convergence_rates_linear() {
    let (e1, h1) = solve_sine_square(0.2, "linear");
    let (e2, h2) = solve_sine_square(0.1, "linear");
    let (e3, h3) = solve_sine_square(0.05, "linear");
    let r12 = (e1 / e2).ln() / (h1 / h2).ln();
    let r23 = (e2 / e3).ln() / (h2 / h3).ln();
    eprintln!("P1 L2 rates: r12={r12:.4} (e1={e1:.3e}, e2={e2:.3e}), r23={r23:.4} (e3={e3:.3e})");
    assert!(r12 >= 1.9, "P1 L2 rate 0.2->0.1 {r12:.4} < 1.9");
    assert!(r23 >= 1.9, "P1 L2 rate 0.1->0.05 {r23:.4} < 1.9");
}

#[test]
fn l2_convergence_rates_quadratic() {
    let (e1, h1) = solve_sine_square(0.2, "quadratic");
    let (e2, h2) = solve_sine_square(0.1, "quadratic");
    let (e3, h3) = solve_sine_square(0.05, "quadratic");
    let r12 = (e1 / e2).ln() / (h1 / h2).ln();
    let r23 = (e2 / e3).ln() / (h2 / h3).ln();
    eprintln!("P2 L2 rates: r12={r12:.4} (e1={e1:.3e}, e2={e2:.3e}), r23={r23:.4} (e3={e3:.3e})");
    assert!(r12 >= 2.8, "P2 L2 rate 0.2->0.1 {r12:.4} < 2.8");
    assert!(r23 >= 2.8, "P2 L2 rate 0.1->0.05 {r23:.4} < 2.8");
}

#[test]
fn function_handles_match_constants_and_hr_dirichlet() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m1 = createpde();\n\
         geometryFromEdges(m1, dl);\n\
         generateMesh(m1, 'Hmax', 0.2, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(m1, 'm', 0, 'd', 0, 'c', [1.5; 0.2; 2.0], 'a', 2, 'f', 3);\n\
         applyBoundaryCondition(m1, 'dirichlet', 'Edge', [1 2], 'u', 1.25);\n\
         applyBoundaryCondition(m1, 'neumann', 'Edge', [3 4], 'g', 0.75, 'q', 0.5);\n\
         r1 = solvepde(m1);\n\
         m2 = createpde();\n\
         geometryFromEdges(m2, dl);\n\
         generateMesh(m2, 'Hmax', 0.2, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(m2, 'm', @(l,s) zeros(1,numel(l.x)), 'd', @(l,s) 0, ...\n\
             'c', @(l,s) repmat([1.5; 0.2; 2.0], 1, numel(l.x)), ...\n\
             'a', @(l,s) 2 + 0*l.x, 'f', @(l,s) 3 + 0*l.y);\n\
         applyBoundaryCondition(m2, 'dirichlet', 'Edge', [1 2], 'h', @(l,s) 2 + 0*l.x, 'r', @(l,s) 2.5 + 0*l.x);\n\
         applyBoundaryCondition(m2, 'neumann', 'Edge', [3 4], 'g', @(l,s) 0.75*(l.nx.^2 + l.ny.^2), 'q', @(l,s) 0.5 + 0*l.x);\n\
         r2 = solvepde(m2);\n\
         diff = max(abs(r1.NodalSolution - r2.NodalSolution));\n"
    ));
    let diff = scalar(&e, "diff");
    assert!(diff <= 1e-12, "handle vs const diff {diff:.3e} > 1e-12");
}

#[test]
fn c_tensor_2x1_and_4x1_nonsymmetric() {
    // 2x1 diagonal [2; 3] matches 4x1 [2; 0; 0; 3], and 4x1 nonsymmetric solves via sparse LU.
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m1 = createpde(); geometryFromEdges(m1, dl);\n\
         generateMesh(m1, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m1, 'm', 0, 'd', 0, 'c', [2; 3], 'a', 0, 'f', 1);\n\
         applyBoundaryCondition(m1, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         r1 = solvepde(m1);\n\
         m2 = createpde(); geometryFromEdges(m2, dl);\n\
         generateMesh(m2, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m2, 'm', 0, 'd', 0, 'c', [2; 0; 0; 3], 'a', 0, 'f', 1);\n\
         applyBoundaryCondition(m2, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         r2 = solvepde(m2);\n\
         diff_diag = max(abs(r1.NodalSolution - r2.NodalSolution));\n\
         % 4x1 nonsymmetric c = [2; 0.3; -0.1; 3] on linear exact solution u = x + 2*y => div(c grad u) = 0
         m3 = createpde(); geometryFromEdges(m3, dl);\n\
         generateMesh(m3, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m3, 'm', 0, 'd', 0, 'c', [2; 0.3; -0.1; 3], 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m3, 'dirichlet', 'Edge', 1:4, 'u', @(l,s) l.x + 2*l.y);\n\
         r3 = solvepde(m3);\n\
         P = r3.Mesh.Nodes;\n\
         err_lin = max(abs(r3.NodalSolution - (P(1,:) + 2*P(2,:))'));\n"
    ));
    assert!(scalar(&e, "diff_diag") <= 1e-14);
    assert!(scalar(&e, "err_lin") <= 1e-12);
}

#[test]
fn override_order_coefficients_and_bcs() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m1 = createpde();\n\
         geometryFromEdges(m1, dl);\n\
         generateMesh(m1, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m1, 'm', 0, 'd', 0, 'c', 99, 'a', 50, 'f', 100);\n\
         ca = specifyCoefficients(m1, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 2, 'Face', 1);\n\
         applyBoundaryCondition(m1, 'dirichlet', 'Edge', 1:4, 'u', 10);\n\
         applyBoundaryCondition(m1, 'neumann', 'Edge', [3 4], 'g', 1, 'q', 0.5);\n\
         applyBoundaryCondition(m1, 'dirichlet', 'Edge', [1 2], 'u', 0);\n\
         r1 = solvepde(m1);\n\
         m2 = createpde();\n\
         geometryFromEdges(m2, dl);\n\
         generateMesh(m2, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m2, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 2);\n\
         applyBoundaryCondition(m2, 'dirichlet', 'Edge', [1 2], 'u', 0);\n\
         applyBoundaryCondition(m2, 'neumann', 'Edge', [3 4], 'g', 1, 'q', 0.5);\n\
         r2 = solvepde(m2);\n\
         diff = max(abs(r1.NodalSolution - r2.NodalSolution));\n\
         n_ca = numel(m1.EquationCoefficients.CoefficientAssignments);\n\
         n_bc = numel(m1.BoundaryConditions.BoundaryConditionAssignments);\n\
         ca_f = ca.f;\n\
         eq_res = double(isequal(r1, r1));\n"
    ));
    assert!(scalar(&e, "diff") <= 1e-14);
    assert_eq!(scalar(&e, "n_ca"), 2.0);
    assert_eq!(scalar(&e, "n_bc"), 3.0);
    assert_eq!(scalar(&e, "ca_f"), 2.0);
    assert_eq!(scalar(&e, "eq_res"), 1.0);
}

#[test]
fn error_cases() {
    // Missing required coefficient name.
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0);"
    ));
    assert!(
        err.contains("'f'") || err.contains("missing"),
        "unexpected err: {err}"
    );

    // No mesh.
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); specifyCoefficients(m,'m',0,'d',0,'c',1,'a',0,'f',1); solvepde(m);"
    ));
    assert!(err.to_lowercase().contains("mesh"), "unexpected err: {err}");

    // No coefficients.
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); generateMesh(m,'Hmax',0.5); solvepde(m);"
    ));
    assert!(
        err.to_lowercase().contains("coefficient"),
        "unexpected err: {err}"
    );

    // Nonzero d / m (time-dependent).
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); generateMesh(m,'Hmax',0.5); specifyCoefficients(m,'m',0,'d',1,'c',1,'a',0,'f',1); solvepde(m);"
    ));
    assert!(
        err.to_lowercase().contains("time-dependent") || err.contains("tlist"),
        "unexpected err: {err}"
    );

    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); generateMesh(m,'Hmax',0.5); specifyCoefficients(m,'m',1,'d',0,'c',1,'a',0,'f',1); solvepde(m);"
    ));
    assert!(
        err.to_lowercase().contains("time-dependent") || err.contains("tlist"),
        "unexpected err: {err}"
    );

    // State-dependent / nonlinear coefficient in time-dependent PDE (still unsupported).
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); generateMesh(m,'Hmax',0.5); \
         specifyCoefficients(m,'m',0,'d',1,'c',1,'a',0,'f',@(l,s) s.u.^2); \
         applyBoundaryCondition(m,'dirichlet','Edge',1:4,'u',0); setInitialConditions(m,0); solvepde(m,[0 0.1]);"
    ));
    assert!(
        err.to_lowercase().contains("nonlinear"),
        "unexpected err: {err}"
    );

    // Read-only results property.
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); generateMesh(m,'Hmax',0.5,'GeometricOrder','linear'); \
         specifyCoefficients(m,'m',0,'d',0,'c',1,'a',0,'f',1); \
         applyBoundaryCondition(m,'dirichlet','Edge',1:4,'u',0); r = solvepde(m); r.NodalSolution = 0;"
    ));
    assert!(
        err.to_lowercase().contains("read-only"),
        "unexpected err: {err}"
    );
}
