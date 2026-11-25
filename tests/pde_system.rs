//! Tests for N-equation 2-D PDE systems, nonlinear stationary solves,
//! nodal recovered gradients (XGradients/YGradients), interpolateSolution,
//! and evaluateGradient (Batch 8b4).
use matrust::{Engine, Matrix};
use std::path::PathBuf;

const UNIT_SQUARE: &str = "\
R1 = [3; 4; 0; 1; 1; 0; 0; 0; 1; 1];\n\
dl = decsg(R1);\n";

const UNIT_DISK: &str = "\
C1 = [1; 0; 0; 1];\n\
dl = decsg(C1);\n";

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

#[test]
fn decoupled_two_equation_system_equals_two_scalar_solves() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m1 = createpde(); geometryFromEdges(m1, dl);\n\
         generateMesh(m1, 'Hmax', 0.25, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(m1, 'm', 0, 'd', 0, 'c', 1.5, 'a', 0.5, 'f', 2);\n\
         applyBoundaryCondition(m1, 'dirichlet', 'Edge', [1 2], 'u', 1);\n\
         applyBoundaryCondition(m1, 'neumann', 'Edge', [3 4], 'g', 0.5, 'q', 0.25);\n\
         r1 = solvepde(m1);\n\
         \n\
         m2 = createpde(); geometryFromEdges(m2, dl);\n\
         generateMesh(m2, 'Hmax', 0.25, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(m2, 'm', 0, 'd', 0, 'c', 2.5, 'a', 1.25, 'f', -3);\n\
         applyBoundaryCondition(m2, 'dirichlet', 'Edge', [1 2], 'u', -2);\n\
         applyBoundaryCondition(m2, 'neumann', 'Edge', [3 4], 'g', -1.0, 'q', 0.75);\n\
         r2 = solvepde(m2);\n\
         \n\
         ms = createpde(2); geometryFromEdges(ms, dl);\n\
         generateMesh(ms, 'Hmax', 0.25, 'GeometricOrder', 'quadratic');\n\
         % 3N form: isotropic c = 1.5 for equation 1 and 2.5 for equation 2.\n\
         specifyCoefficients(ms, 'm', 0, 'd', 0, 'c', [1.5; 0; 1.5; 2.5; 0; 2.5], 'a', [0.5; 1.25], 'f', [2; -3]);\n\
         applyBoundaryCondition(ms, 'dirichlet', 'Edge', [1 2], 'u', [1; -2]);\n\
         applyBoundaryCondition(ms, 'neumann', 'Edge', [3 4], 'g', [0.5; -1.0], 'q', [0.25; 0.75]);\n\
         rs = solvepde(ms);\n\
         \n\
         err1 = max(abs(rs.NodalSolution(:, 1) - r1.NodalSolution));\n\
         err2 = max(abs(rs.NodalSolution(:, 2) - r2.NodalSolution));\n\
         sz = size(rs.NodalSolution);\n"
    ));
    let sz = mat(&e, "sz");
    assert_eq!(sz.get(0, 1), 2.0, "NodalSolution must be N x 2");
    assert!(
        scalar(&e, "err1") <= 1e-12,
        "component 1 differs by {}",
        scalar(&e, "err1")
    );
    assert!(
        scalar(&e, "err2") <= 1e-12,
        "component 2 differs by {}",
        scalar(&e, "err2")
    );
}

#[test]
#[allow(clippy::needless_range_loop)]
fn coupled_a_matrix_system_matches_skfem_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("pde")
        .join("square_system2_quadratic.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read fixture {}: {err}", path.display()));
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let fx_u: Vec<Vec<f64>> = serde_json::from_value(v["u"].clone()).unwrap();

    // Non-symmetric coupled a-matrix: A = [2, -0.5; 1, 3] => column-major [2; 1; -0.5; 3].
    // c = [1; 2] is the documented 2-element form for N = 2: diag(1, 2) for both equations.
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde(2);\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.15, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', [1; 2], 'a', [2; 1; -0.5; 3], 'f', [1; 2]);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', [0; 1]);\n\
         res = solvepde(m);\n\
         U = res.NodalSolution;\n"
    ));
    let u = mat(&e, "U");
    assert_eq!(u.rows(), fx_u.len());
    assert_eq!(u.cols(), 2);
    let mut max_diff = 0.0f64;
    for (i, row) in fx_u.iter().enumerate() {
        for c in 0..2 {
            max_diff = max_diff.max((u.get(i, c) - row[c]).abs());
        }
    }
    eprintln!("coupled 2-equation system vs scikit-fem: {max_diff:.3e}");
    assert!(
        max_diff <= 1e-8,
        "coupled 2-equation system differs from scikit-fem fixture by {max_diff:.3e} (> 1e-8)"
    );
}

#[test]
fn system_c_forms_and_mixed_equation_bcs() {
    // Test handle-returned 4N, 3N, 4N, N(2N+1), and 4N^2 forms of c for N = 2, plus symmetric N(N+1)/2 for a,
    // and 'Equation'/'EquationIndex' and 'mixed' boundary conditions.
    let e = run(&format!(
        "{UNIT_SQUARE}\
         % 1) Equivalent c representations (MathWorks system layout) of the block-diagonal tensor\n\
         %    c(1,1,:,:) = diag(1,2), c(2,2,:,:) = diag(3,4). (The 2N form [1;2;3;4] is the\n\
         %    4-element form for N = 2, so it is not used here.)\n\
         %    3N: [1;0;2;3;0;4], 4N: [1;0;0;2;3;0;0;4]\n\
         %    N(2N+1) block-wise: block (1,1) [1;0;2], block (1,2) [0;0;0;0], block (2,2) [3;0;4]\n\
         %    4N^2 block-wise: block (1,1), (2,1), (1,2), (2,2), each column-major\n\
         c_2n = @(l, s) repmat([1; 0; 0; 2; 3; 0; 0; 4], 1, numel(l.x)); % 4N rows from a handle\n\
         c_3n = [1; 0; 2; 3; 0; 4];\n\
         c_4n = [1; 0; 0; 2; 3; 0; 0; 4];\n\
         c_sym = [1; 0; 2; 0; 0; 0; 0; 3; 0; 4];\n\
         c_full = [1; 0; 0; 2; 0; 0; 0; 0; 0; 0; 0; 0; 3; 0; 0; 4];\n\
         a_sym = [2; 0.5; 3]; % symmetric [2 0.5; 0.5 3]\n\
         \n\
         m1 = createpde(2); geometryFromEdges(m1, dl); generateMesh(m1, 'Hmax', 0.3, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m1, 'm', 0, 'd', 0, 'c', c_2n, 'a', a_sym, 'f', [1; 2]);\n\
         applyBoundaryCondition(m1, 'dirichlet', 'Edge', 1:4, 'u', [0; 0]);\n\
         r1 = solvepde(m1);\n\
         \n\
         m2 = createpde(2); geometryFromEdges(m2, dl); generateMesh(m2, 'Hmax', 0.3, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m2, 'm', 0, 'd', 0, 'c', c_3n, 'a', a_sym, 'f', [1; 2]);\n\
         applyBoundaryCondition(m2, 'dirichlet', 'Edge', 1:4, 'u', [0; 0]);\n\
         r2 = solvepde(m2);\n\
         \n\
         m3 = createpde(2); geometryFromEdges(m3, dl); generateMesh(m3, 'Hmax', 0.3, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m3, 'm', 0, 'd', 0, 'c', c_4n, 'a', a_sym, 'f', [1; 2]);\n\
         applyBoundaryCondition(m3, 'dirichlet', 'Edge', 1:4, 'u', [0; 0]);\n\
         r3 = solvepde(m3);\n\
         \n\
         m4 = createpde(2); geometryFromEdges(m4, dl); generateMesh(m4, 'Hmax', 0.3, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m4, 'm', 0, 'd', 0, 'c', c_sym, 'a', a_sym, 'f', [1; 2]);\n\
         applyBoundaryCondition(m4, 'dirichlet', 'Edge', 1:4, 'u', [0; 0]);\n\
         r4 = solvepde(m4);\n\
         \n\
         m5 = createpde(2); geometryFromEdges(m5, dl); generateMesh(m5, 'Hmax', 0.3, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m5, 'm', 0, 'd', 0, 'c', c_full, 'a', a_sym, 'f', [1; 2]);\n\
         applyBoundaryCondition(m5, 'dirichlet', 'Edge', 1:4, 'u', [0; 0]);\n\
         r5 = solvepde(m5);\n\
         \n\
         d2 = max(abs(r2.NodalSolution(:) - r1.NodalSolution(:)));\n\
         d3 = max(abs(r3.NodalSolution(:) - r1.NodalSolution(:)));\n\
         d4 = max(abs(r4.NodalSolution(:) - r1.NodalSolution(:)));\n\
         d5 = max(abs(r5.NodalSolution(:) - r1.NodalSolution(:)));\n\
         \n\
         % 2) 'Equation' / 'mixed' BC: u1 = 3 on all edges, u2 = x (Dirichlet u2=0 on x=0, g2=1 on x=1)\n\
         mm = createpde(2); geometryFromEdges(mm, dl); generateMesh(mm, 'Hmax', 0.3, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(mm, 'm', 0, 'd', 0, 'c', [1; 1], 'a', 0, 'f', [0; 0]);\n\
         applyBoundaryCondition(mm, 'dirichlet', 'Edge', [1 3], 'u', 3, 'Equation', 1);\n\
         applyBoundaryCondition(mm, 'dirichlet', 'Edge', 4, 'u', [3; 0]);\n\
         applyBoundaryCondition(mm, 'mixed', 'Edge', 2, 'u', 3, 'EquationIndex', 1, 'g', [0; 1]);\n\
         rm = solvepde(mm);\n\
         P = rm.Mesh.Nodes;\n\
         err_u1 = max(abs(rm.NodalSolution(:, 1) - 3));\n\
         err_u2 = max(abs(rm.NodalSolution(:, 2) - P(1, :)'));\n"
    ));
    assert!(scalar(&e, "d2") <= 1e-13);
    assert!(scalar(&e, "d3") <= 1e-13);
    assert!(scalar(&e, "d4") <= 1e-13);
    assert!(scalar(&e, "d5") <= 1e-13);
    assert!(
        scalar(&e, "err_u1") <= 1e-12,
        "mixed BC u1 err {}",
        scalar(&e, "err_u1")
    );
    assert!(
        scalar(&e, "err_u2") <= 1e-12,
        "mixed BC u2 err {}",
        scalar(&e, "err_u2")
    );
}

#[test]
fn system_time_dependent_and_eigen_shapes_and_values() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         % Time-dependent 2-equation decoupled heat system: N x 2 x Nt\n\
         mt = createpde(2); geometryFromEdges(mt, dl);\n\
         generateMesh(mt, 'Hmax', 0.3, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(mt, 'm', 0, 'd', [1; 2], 'c', [1; 1], 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(mt, 'dirichlet', 'Edge', 1:4, 'u', [0; 0]);\n\
         setInitialConditions(mt, @(loc) [sin(pi*loc.x).*sin(pi*loc.y); 2*sin(pi*loc.x).*sin(pi*loc.y)]);\n\
         rt = solvepde(mt, [0 0.05 0.1]);\n\
         sz_t = size(rt.NodalSolution);\n\
         sz_gx = size(rt.XGradients);\n\
         sz_gy = size(rt.YGradients);\n\
         \n\
         % Eigenvalue 2-equation decoupled system: N x 2 x k\n\
         me = createpde(2); geometryFromEdges(me, dl);\n\
         generateMesh(me, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(me, 'm', 0, 'd', [1; 2], 'c', [1; 1], 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(me, 'dirichlet', 'Edge', 1:4, 'u', [0; 0]);\n\
         re = solvepdeeig(me, [0 30]);\n\
         sz_e = size(re.Eigenvectors);\n\
         lam = re.Eigenvalues;\n"
    ));
    let sz_t = mat(&e, "sz_t");
    assert_eq!(
        sz_t.cols(),
        3,
        "TimeDependentResults.NodalSolution must be 3-D for N>1"
    );
    assert_eq!(sz_t.get(0, 1), 2.0);
    assert_eq!(sz_t.get(0, 2), 3.0);
    assert_eq!(mat(&e, "sz_gx").data(), sz_t.data());
    assert_eq!(mat(&e, "sz_gy").data(), sz_t.data());

    let sz_e = mat(&e, "sz_e");
    let lam = mat(&e, "lam");
    assert_eq!(
        sz_e.cols(),
        3,
        "EigenResults.Eigenvectors must be 3-D for N>1"
    );
    assert_eq!(sz_e.get(0, 1), 2.0);
    assert_eq!(sz_e.get(0, 2), lam.rows() as f64);
    // Lowest eigenvalues: pi^2 (from d=2: 2*pi^2/2 = pi^2 ~ 9.87) and 2*pi^2 (from d=1: ~ 19.74).
    assert!(lam.rows() >= 2);
    assert!(
        (lam.get(0, 0) - std::f64::consts::PI.powi(2)).abs() / std::f64::consts::PI.powi(2) < 0.05
    );
}

#[test]
fn nonlinear_stationary_unit_disk_and_manufactured_solution() {
    // 1) -div((1 + u^2) grad u) = 1 on unit disk, u = 0 on boundary.
    // Verify SolverOptions defaults, convergence, positivity/symmetry, and discrete residual <= ResidualTolerance.
    let e = run(&format!(
        "{UNIT_DISK}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.2, 'GeometricOrder', 'quadratic');\n\
         rtol_def = m.SolverOptions.ResidualTolerance;\n\
         maxit_def = m.SolverOptions.MaxIterations;\n\
         minstep_def = m.SolverOptions.MinStep;\n\
         rep_def = m.SolverOptions.ReportStatistics;\n\
         m.SolverOptions.ResidualTolerance = 1e-10;\n\
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', @(l, s) 1 + s.u.^2, 'a', 0, 'f', 1);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         res = solvepde(m);\n\
         u = res.NodalSolution;\n\
         u_max = max(u);\n\
         u_min = min(u);\n\
         \n\
         % Re-solve linearly with c frozen at 1 + u_sol^2: if u_sol satisfies the discrete\n\
         % nonlinear system A(u_sol)*u_sol = b, then the frozen-c linear solve reproduces u_sol.\n\
         u_frozen = interpolateSolution(res, 0, 0);\n\
         ml = createpde(); geometryFromEdges(ml, dl);\n\
         generateMesh(ml, 'Hmax', 0.2, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(ml, 'm', 0, 'd', 0, 'c', @(l, s) 1 + interpolateSolution(res, l.x, l.y)'.^2, 'a', 0, 'f', 1);\n\
         applyBoundaryCondition(ml, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         rl = solvepde(ml);\n\
         fixed_pt_err = max(abs(rl.NodalSolution - u));\n"
    ));
    assert_eq!(scalar(&e, "rtol_def"), 1e-4);
    assert_eq!(scalar(&e, "maxit_def"), 25.0);
    assert!((scalar(&e, "minstep_def") - 1.0 / 65536.0).abs() <= 1e-15);
    assert!(scalar(&e, "u_max") > 0.2 && scalar(&e, "u_max") < 0.25);
    assert!(scalar(&e, "u_min") >= -1e-14);
    assert!(
        scalar(&e, "fixed_pt_err") <= 1e-9,
        "nonlinear solution does not satisfy discrete fixed-point equation: err = {:.3e}",
        scalar(&e, "fixed_pt_err")
    );

    // 2) Manufactured solution on unit square:
    //    u_ex = sin(pi x) sin(pi y), c(u) = 1 + u^2
    //    -div((1 + u^2) grad u) = (1 + u^2)*2*pi^2*u - 2*u*(ux^2 + uy^2)
    let e2 = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.15, 'GeometricOrder', 'quadratic');\n\
         m.SolverOptions.ResidualTolerance = 1e-10;\n\
         f_man = @(l, s) (1 + (sin(pi*l.x).*sin(pi*l.y)).^2) .* (2*pi^2*sin(pi*l.x).*sin(pi*l.y)) ...\n\
                         - 2*(sin(pi*l.x).*sin(pi*l.y)) .* ((pi*cos(pi*l.x).*sin(pi*l.y)).^2 + (pi*sin(pi*l.x).*cos(pi*l.y)).^2);\n\
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', @(l, s) 1 + s.u.^2, 'a', 0, 'f', f_man);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         res = solvepde(m);\n\
         P = res.Mesh.Nodes;\n\
         u_ex = (sin(pi*P(1,:)).*sin(pi*P(2,:)))';\n\
         max_err = max(abs(res.NodalSolution - u_ex));\n"
    ));
    assert!(
        scalar(&e2, "max_err") <= 1e-3,
        "manufactured nonlinear error {:.3e} > 1e-3",
        scalar(&e2, "max_err")
    );
}

#[test]
fn gradients_and_interpolation_exactness_and_nan_outside() {
    // 1) Linear u = 2x + 3y reproduced exactly (<= 1e-12) by XGradients/YGradients and evaluateGradient.
    let e_lin = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde(); geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', @(l,s) 2*l.x + 3*l.y);\n\
         r = solvepde(m);\n\
         err_gx = max(abs(r.XGradients - 2));\n\
         err_gy = max(abs(r.YGradients - 3));\n\
         xq = [0.13, 0.47, 0.81, 0.29, -0.5, 1.5];\n\
         yq = [0.22, 0.63, 0.19, 0.77, 0.5, 0.5];\n\
         [gx, gy] = evaluateGradient(r, xq, yq);\n\
         err_egx = max(abs(gx(1:4) - 2));\n\
         err_egy = max(abs(gy(1:4) - 3));\n\
         nan_out_g = double(all(isnan(gx(5:6))) && all(isnan(gy(5:6))));\n"
    ));
    eprintln!(
        "gradients P1: gx {:.3e} gy {:.3e} egx {:.3e} egy {:.3e}",
        scalar(&e_lin, "err_gx"),
        scalar(&e_lin, "err_gy"),
        scalar(&e_lin, "err_egx"),
        scalar(&e_lin, "err_egy")
    );
    assert!(scalar(&e_lin, "err_gx") <= 1e-12);
    assert!(scalar(&e_lin, "err_gy") <= 1e-12);
    assert!(scalar(&e_lin, "err_egx") <= 1e-12);
    assert!(scalar(&e_lin, "err_egy") <= 1e-12);
    assert_eq!(scalar(&e_lin, "nan_out_g"), 1.0);

    // 2) Quadratic field u = 1 + 2x - 3y + 4x^2 - 5xy + 6y^2 on a quadratic (P2) mesh:
    //    -Delta u = -(8 + 12) = -20 => f = -20, Dirichlet u = u_ex on boundary.
    //    interpolateSolution must be exact (<= 1e-12) at interior points and NaN outside.
    let e_quad = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde(); geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.25, 'GeometricOrder', 'quadratic');\n\
         u_fn = @(x, y) 1 + 2*x - 3*y + 4*x.^2 - 5*x.*y + 6*y.^2;\n\
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', -20);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', @(l,s) u_fn(l.x, l.y));\n\
         r = solvepde(m);\n\
         xq = [0.11, 0.37, 0.64, 0.89, 0.52, 0.23, 0.71, -0.1, 1.2];\n\
         yq = [0.17, 0.83, 0.41, 0.26, 0.55, 0.68, 0.91, 0.5, 0.5];\n\
         ui = interpolateSolution(r, xq, yq);\n\
         u_ex = u_fn(xq(1:7), yq(1:7))';\n\
         err_u = max(abs(ui(1:7) - u_ex));\n\
         nan_out_u = double(all(isnan(ui(8:9))));\n\
         [gx, gy] = evaluateGradient(r, xq(1:7), yq(1:7));\n\
         err_gx = max(abs(gx - (2 + 8*xq(1:7) - 5*yq(1:7))'));\n\
         err_gy = max(abs(gy - (-3 - 5*xq(1:7) + 12*yq(1:7))'));\n"
    ));
    assert!(
        scalar(&e_quad, "err_u") <= 1e-12,
        "quadratic interpolateSolution err {:.3e} > 1e-12",
        scalar(&e_quad, "err_u")
    );
    assert_eq!(scalar(&e_quad, "nan_out_u"), 1.0);
    eprintln!(
        "interpolation/gradients P2: u {:.3e} gx {:.3e} gy {:.3e}",
        scalar(&e_quad, "err_u"),
        scalar(&e_quad, "err_gx"),
        scalar(&e_quad, "err_gy")
    );
    assert!(
        scalar(&e_quad, "err_gx") <= 1e-12,
        "quadratic evaluateGradient gx err {:.3e} > 1e-12",
        scalar(&e_quad, "err_gx")
    );
    assert!(
        scalar(&e_quad, "err_gy") <= 1e-12,
        "quadratic evaluateGradient gy err {:.3e} > 1e-12",
        scalar(&e_quad, "err_gy")
    );

    // 3) System + time-dependent interpolateSolution(res, xq, yq, iU, iT).
    let e_sys = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde(2); geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.3, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', @(l,s) [l.x; 2*l.y]);\n\
         setInitialConditions(m, @(l) [l.x; 2*l.y]);\n\
         r = solvepde(m, [0 0.05 0.1]);\n\
         u2_t3 = interpolateSolution(r, [0.25 0.75], [0.4 0.6], 2, 3);\n\
         err_sys_interp = max(abs(u2_t3 - [0.8; 1.2]));\n\
         err_t_gx = max(abs(r.XGradients(:, 1, 3) - 1));\n\
         err_t_gy = max(abs(r.YGradients(:, 2, 3) - 2));\n"
    ));
    eprintln!(
        "system time interp/gradients: u {:.3e} gx {:.3e} gy {:.3e}",
        scalar(&e_sys, "err_sys_interp"),
        scalar(&e_sys, "err_t_gx"),
        scalar(&e_sys, "err_t_gy")
    );
    assert!(scalar(&e_sys, "err_sys_interp") <= 1e-11);
    assert!(scalar(&e_sys, "err_t_gx") <= 1e-11);
    assert!(scalar(&e_sys, "err_t_gy") <= 1e-11);
}

#[test]
fn system_and_postproc_error_cases() {
    // Invalid c row count for N = 2 (valid: 1, 2, 3, 4, 6, 8, 10, 16).
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(2); geometryFromEdges(m, dl); \
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', [1; 2; 3; 4; 5], 'a', 0, 'f', [0; 0]);"
    ));
    assert!(
        err.contains("row count") || err.contains("'c'"),
        "unexpected err: {err}"
    );

    // Out-of-range EquationIndex.
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(2); geometryFromEdges(m, dl); \
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1, 'u', 0, 'EquationIndex', 3);"
    ));
    assert!(err.contains("exceeds"), "unexpected err: {err}");

    // Verify new help examples execute cleanly.
    let eh = run(
        "R1=[3;4;0;1;1;0;0;0;1;1]; dl=decsg(R1); m=createpde(2); geometryFromEdges(m,dl); \
         ca=specifyCoefficients(m,'m',0,'d',0,'c',[1;0;1;2;0;2],'a',[1;0;1],'f',[1;2]); a1 = numel(ca.c); \
         bc=applyBoundaryCondition(m,'mixed','Edge',1:4,'u',0,'EquationIndex',1,'g',[0;1]); a2 = numel(bc.EquationIndex); \
         ms=createpde(); geometryFromEdges(ms,dl); generateMesh(ms,'Hmax',0.3,'GeometricOrder','quadratic'); \
         specifyCoefficients(ms,'m',0,'d',0,'c',1,'a',0,'f',0); \
         applyBoundaryCondition(ms,'dirichlet','Edge',1:4,'u',@(l,s) 2*l.x+3*l.y); \
         res=solvepde(ms); \
         a3 = double(abs(interpolateSolution(res, 0.5, 0.25) - 1.75) < 1e-12); \
         [gx, gy] = evaluateGradient(res, 0.5, 0.25); \
         a4 = double(abs(gx - 2) < 1e-12 && abs(gy - 3) < 1e-12);",
    );
    assert_eq!(scalar(&eh, "a1"), 6.0);
    assert_eq!(scalar(&eh, "a2"), 1.0);
    assert_eq!(scalar(&eh, "a3"), 1.0);
    assert_eq!(scalar(&eh, "a4"), 1.0);
}

#[test]
fn eigen_rejects_nonsymmetric_a_d_and_q() {
    let mesh = format!(
        "{UNIT_SQUARE}\
         m = createpde(2); geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.5, 'GeometricOrder', 'linear');\n"
    );
    // Column-major [2; 0; 1; 3] is [2 1; 0 3], not symmetric.
    let err_a = error(&format!(
        "{mesh}\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', [2; 0; 1; 3], 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         solvepdeeig(m, [0 40]);"
    ));
    assert!(
        err_a.contains("solvepdeeig: Nonsymmetric coefficient 'a' is not supported."),
        "unexpected err: {err_a}"
    );

    let err_d = error(&format!(
        "{mesh}\
         specifyCoefficients(m, 'm', 0, 'd', [2; 0; 1; 3], 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         solvepdeeig(m, [0 40]);"
    ));
    assert!(
        err_d.contains("solvepdeeig: Nonsymmetric coefficient 'd' is not supported."),
        "unexpected err: {err_d}"
    );

    let err_q = error(&format!(
        "{mesh}\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'neumann', 'Edge', 1:4, 'g', 0, 'q', [2; 0; 1; 3]);\n\
         solvepdeeig(m, [0 40]);"
    ));
    assert!(
        err_q.contains("solvepdeeig: Nonsymmetric coefficient 'q' is not supported."),
        "unexpected err: {err_q}"
    );

    // The symmetric full form of the same diagonal magnitudes must still solve.
    let e = run(&format!(
        "{mesh}\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', [2; 1; 1; 3], 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         r = solvepdeeig(m, [0 40]);\n\
         nlam = numel(r.Eigenvalues);\n\
         lam1 = r.Eigenvalues(1);"
    ));
    assert!(scalar(&e, "nlam") >= 1.0);
    assert!(scalar(&e, "lam1").is_finite());
}

#[test]
fn coeff_workspace_rejects_large_system_on_small_mesh() {
    // N = 1024 on a coarse unit square: even a minimum triangulation has
    // n_dom * 4 * N^2 above the 2^27 coefficient-workspace cap.
    let err = error(&format!(
        "{UNIT_SQUARE}\
         m = createpde(1024); geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         solvepde(m);"
    ));
    assert!(
        err.contains("system too large") && err.contains("coefficient workspace exceeds"),
        "unexpected err: {err}"
    );
}

#[test]
fn eigen_single_mode_keeps_component_index() {
    // d = [1; 2] puts exactly one eigenvalue (the d = 2 mode, about pi^2) in [0, 15].
    // Eigenvectors then collapse from N x 2 x 1 to N x 2; equation index 2 must still work.
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde(2); geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.3, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m, 'm', 0, 'd', [1; 2], 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         r = solvepdeeig(m, [0 15]);\n\
         nlam = numel(r.Eigenvalues);\n\
         u1 = interpolateSolution(r, 0.5, 0.5, 2, 1);\n\
         u0 = interpolateSolution(r, 0.5, 0.5, 1, 1);\n"
    ));
    assert_eq!(
        scalar(&e, "nlam"),
        1.0,
        "range [0 15] should hold one eigenvalue"
    );
    let u0 = scalar(&e, "u0");
    let u1 = scalar(&e, "u1");
    assert!(u0.is_finite() && u1.is_finite(), "u0={u0} u1={u1}");
    assert!(u0.abs() < 1e-8, "idle component leaked: {u0}");
    assert!(
        u1.abs() > 0.2,
        "component 2 of the single mode should be nonzero at the center, got {u1}"
    );
}

/// Max abs difference between column `col` of `a` and the single column of `b`.
fn col_diff(a: &Matrix, col: usize, b: &Matrix) -> f64 {
    (0..a.rows())
        .map(|i| (a.get(i, col) - b.get(i, 0)).abs())
        .fold(0.0, f64::max)
}

#[test]
fn system_c_short_forms_follow_matlab_precedence() {
    // MathWorks "c Coefficient for Systems": for N = 2, 3, 4 the 2-, 3-, 4-element forms take
    // precedence over the N-element form, and for N = 2 the 4-element form over the 2N form.
    // A short form puts the same 2x2 block on every equation, so with a = 0 each component
    // equals the scalar solve with that c.
    let solve = |n: usize, c: &str, f: &str, u: &str| {
        format!(
            "m = createpde({n}); geometryFromEdges(m, dl); generateMesh(m, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
             specifyCoefficients(m, 'm', 0, 'd', 0, 'c', {c}, 'a', 0, 'f', {f});\n\
             applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', {u});\n\
             r = solvepde(m);\n"
        )
    };
    let cases = [
        (2, "[1; 2]", "[1; 1]", "[0; 0]"),
        (2, "[1; 0.3; 0.3; 2]", "[1; 1]", "[0; 0]"),
        (3, "[1; 0.3; 2]", "[1; 1; 1]", "[0; 0; 0]"),
        (4, "[1; 0.2; -0.1; 2]", "[1; 1; 1; 1]", "[0; 0; 0; 0]"),
    ];
    for (n, c, f, u) in cases {
        let e = run(&format!(
            "{UNIT_SQUARE}{}U = r.NodalSolution;\n{}S = r.NodalSolution;\n{}S1 = r.NodalSolution;\n",
            solve(n, c, f, u),
            solve(1, c, "1", "0"),
            solve(1, "1", "1", "0"),
        ));
        let (uu, ss, s1) = (mat(&e, "U"), mat(&e, "S"), mat(&e, "S1"));
        assert_eq!(uu.cols(), n);
        for comp in 0..n {
            let d = col_diff(&uu, comp, &ss);
            assert!(
                d <= 1e-12,
                "N={n} c={c}: component {} differs from scalar c solve by {d:.3e}",
                comp + 1
            );
        }
        // The old reading (N-element / 2N form) differs visibly from the documented one.
        if c == "[1; 2]" {
            let old1 = col_diff(&uu, 0, &s1);
            assert!(
                old1 > 1e-2,
                "c=[1;2] equation 1 equals the c=1 solve (diff {old1:.3e})"
            );
        }
    }
}

#[test]
#[allow(clippy::needless_range_loop)]
fn system_c_vector_forms_match_skfem_reference() {
    // Fixture from tool/refs/pde/make_fixtures.sh mode `square_system_cforms`: scikit-fem with
    // the full c(i,j,k,l) tensor built from each documented vector form.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("pde")
        .join("square_system_cforms_quadratic.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read fixture {}: {err}", path.display()));
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let nodes: Vec<Vec<f64>> = serde_json::from_value(v["nodes"].clone()).unwrap();
    let cases = v["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 13);
    let col = |xs: &[f64]| {
        let parts: Vec<String> = xs.iter().map(|x| format!("{x:?}")).collect();
        format!("[{}]", parts.join("; "))
    };
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let n = case["n"].as_u64().unwrap() as usize;
        let c: Vec<f64> = serde_json::from_value(case["c"].clone()).unwrap();
        let a: Vec<Vec<f64>> = serde_json::from_value(case["a"].clone()).unwrap();
        let f: Vec<f64> = serde_json::from_value(case["f"].clone()).unwrap();
        let ud: Vec<f64> = serde_json::from_value(case["dirichlet"].clone()).unwrap();
        let fx_u: Vec<Vec<f64>> = serde_json::from_value(case["u"].clone()).unwrap();
        let a_colmajor: Vec<f64> = (0..n)
            .flat_map(|j| a.iter().map(move |row| row[j]))
            .collect();
        let e = run(&format!(
            "{UNIT_SQUARE}\
             m = createpde({n}); geometryFromEdges(m, dl);\n\
             generateMesh(m, 'Hmax', 0.25, 'GeometricOrder', 'quadratic');\n\
             specifyCoefficients(m, 'm', 0, 'd', 0, 'c', {}, 'a', {}, 'f', {});\n\
             applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', {});\n\
             res = solvepde(m);\n\
             U = res.NodalSolution;\n\
             P = res.Mesh.Nodes;\n",
            col(&c),
            col(&a_colmajor),
            col(&f),
            col(&ud),
        ));
        let p = mat(&e, "P");
        assert_eq!(p.cols(), nodes[0].len(), "{name}: node count");
        let node_diff = (0..p.cols())
            .map(|j| {
                (p.get(0, j) - nodes[0][j])
                    .abs()
                    .max((p.get(1, j) - nodes[1][j]).abs())
            })
            .fold(0.0, f64::max);
        assert!(
            node_diff <= 1e-12,
            "{name}: mesh differs from fixture by {node_diff:.3e}"
        );
        let u = mat(&e, "U");
        assert_eq!(u.cols(), n);
        let mut max_diff = 0.0f64;
        for (i, row) in fx_u.iter().enumerate() {
            for k in 0..n {
                max_diff = max_diff.max((u.get(i, k) - row[k]).abs());
            }
        }
        eprintln!("cforms {name}: max diff {max_diff:.3e}");
        assert!(
            max_diff <= 1e-8,
            "{name}: differs from scikit-fem by {max_diff:.3e} (> 1e-8)"
        );
    }
}

#[test]
fn later_equation_index_bc_replaces_whole_edge_assignment() {
    // MathWorks applyBoundaryCondition: the last assignment to a region wins, and with
    // EquationIndex the remaining components get the default zero Neumann condition.
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde(2); geometryFromEdges(m, dl); generateMesh(m, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', [1; 1]);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', [0; 0]);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1, 'u', 0, 'EquationIndex', 1);\n\
         r = solvepde(m); U = r.NodalSolution;\n\
         s = createpde(); geometryFromEdges(s, dl); generateMesh(s, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(s, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 1);\n\
         applyBoundaryCondition(s, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         r = solvepde(s); S_all = r.NodalSolution;\n\
         s2 = createpde(); geometryFromEdges(s2, dl); generateMesh(s2, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(s2, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 1);\n\
         applyBoundaryCondition(s2, 'dirichlet', 'Edge', 2:4, 'u', 0);\n\
         r = solvepde(s2); S_234 = r.NodalSolution;\n"
    ));
    let (u, s_all, s_234) = (mat(&e, "U"), mat(&e, "S_all"), mat(&e, "S_234"));
    let d1 = col_diff(&u, 0, &s_all);
    let d2 = col_diff(&u, 1, &s_234);
    let d2_old = col_diff(&u, 1, &s_all);
    assert!(
        d1 <= 1e-12,
        "component 1 differs from all-Dirichlet solve by {d1:.3e}"
    );
    assert!(
        d2 <= 1e-12,
        "component 2 differs from Dirichlet-on-2:4 solve by {d2:.3e}"
    );
    assert!(
        d2_old > 1e-2,
        "component 2 still Dirichlet on edge 1 (diff {d2_old:.3e})"
    );
}
