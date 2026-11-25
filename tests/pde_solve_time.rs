//! Tests for setInitialConditions, time-dependent solvepde (parabolic and
//! hyperbolic), pde.TimeDependentResults, SolverOptions, and IsTimeDependent
//! recomputation (Batch 8b2).
use matrust::{Engine, Matrix};
use std::f64::consts::PI;
use std::path::PathBuf;

const UNIT_SQUARE: &str = "\
R1 = [3; 4; 0; 1; 1; 0; 0; 0; 1; 1];\n\
dl = decsg(R1);\n";

const TWO_FACES: &str = "\
R1 = [3; 4; 0; 1; 1; 0; 0; 0; 1; 1];\n\
R2 = [3; 4; 1; 2; 2; 1; 0; 0; 1; 1];\n\
gd = [R1 R2]; ns = ['R1';'R2']'; dl = decsg(gd, 'R1+R2', ns);\n";

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

#[allow(clippy::type_complexity)]
fn load_parabolic_fixture(name: &str) -> (Vec<Vec<f64>>, Vec<Vec<usize>>, Vec<f64>, Vec<Vec<f64>>) {
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
    let tlist: Vec<f64> = serde_json::from_value(v["tlist"].clone()).unwrap();
    let u: Vec<Vec<f64>> = serde_json::from_value(v["u"].clone()).unwrap();
    (nodes, elements, tlist, u)
}

#[test]
fn heat_series_unit_square_quadratic() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.1, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         setInitialConditions(m, @(loc) sin(pi*loc.x).*sin(pi*loc.y));\n\
         m.SolverOptions.RelativeTolerance = 1e-6;\n\
         m.SolverOptions.AbsoluteTolerance = 1e-9;\n\
         tlist = 0:0.05:0.2;\n\
         res = solvepde(m, tlist);\n\
         P = res.Mesh.Nodes;\n\
         U = res.NodalSolution;\n\
         T = res.SolutionTimes;\n\
         is_td = double(isa(res, 'pde.TimeDependentResults'));\n"
    ));

    assert_eq!(scalar(&e, "is_td"), 1.0);
    let p = mat(&e, "P");
    let u = mat(&e, "U");
    let t = mat(&e, "T");
    let n_nodes = p.cols();
    assert_eq!(u.rows(), n_nodes);
    assert_eq!(u.cols(), 5);
    assert_eq!(t.rows(), 1);
    assert_eq!(t.cols(), 5);

    let mut max_err = 0.0f64;
    for k in 0..5 {
        let tk = t.get(0, k);
        let decay = (-2.0 * PI * PI * tk).exp();
        for i in 0..n_nodes {
            let x = p.get(0, i);
            let y = p.get(1, i);
            let exact = decay * (PI * x).sin() * (PI * y).sin();
            max_err = max_err.max((u.get(i, k) - exact).abs());
        }
    }
    eprintln!("heat series max abs error {max_err:.3e}");
    assert!(
        max_err <= 1e-3,
        "heat series max abs error {max_err:.3e} > 1e-3"
    );
}

#[test]
#[allow(clippy::needless_range_loop)]
fn fixture_square_parabolic_quadratic_matches_skfem() {
    let (fx_nodes, fx_elems, fx_tlist, fx_u) = load_parabolic_fixture("square_parabolic_quadratic");
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.15, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1.5, 'a', 2, 'f', 3);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', [1 2], 'u', 1);\n\
         applyBoundaryCondition(m, 'neumann', 'Edge', [3 4], 'g', 1, 'q', 0.5);\n\
         setInitialConditions(m, 1);\n\
         m.SolverOptions.RelativeTolerance = 1e-8;\n\
         m.SolverOptions.AbsoluteTolerance = 1e-10;\n\
         res = solvepde(m, [0 0.05 0.1 0.2]);\n\
         P = res.Mesh.Nodes; E = res.Mesh.Elements; U = res.NodalSolution; T = res.SolutionTimes;\n"
    ));

    let p = mat(&e, "P");
    let elems = mat(&e, "E");
    let u = mat(&e, "U");
    let t = mat(&e, "T");

    assert_eq!(p.cols(), fx_nodes[0].len());
    assert_eq!(elems.cols(), fx_elems[0].len());
    assert_eq!(t.cols(), fx_tlist.len());
    assert_eq!(u.rows(), fx_u.len());
    assert_eq!(u.cols(), fx_tlist.len());

    let mut max_diff = 0.0f64;
    for i in 0..u.rows() {
        for k in 0..u.cols() {
            max_diff = max_diff.max((u.get(i, k) - fx_u[i][k]).abs());
        }
    }
    eprintln!("square_parabolic_quadratic max_diff vs skfem_ref: {max_diff:.3e}");
    assert!(
        max_diff <= 1e-5,
        "parabolic fixture NodalSolution differs from scikit-fem by {max_diff:.3e} (> 1e-5)"
    );
}

#[test]
fn wave_equation_unit_square() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.12, 'GeometricOrder', 'quadratic');\n\
         specifyCoefficients(m, 'm', 1, 'd', 0, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         setInitialConditions(m, @(loc) sin(pi*loc.x).*sin(pi*loc.y), 0);\n\
         m.SolverOptions.RelativeTolerance = 1e-6;\n\
         m.SolverOptions.AbsoluteTolerance = 1e-9;\n\
         tlist = [0 0.25 0.5];\n\
         res = solvepde(m, tlist);\n\
         P = res.Mesh.Nodes;\n\
         U = res.NodalSolution;\n"
    ));

    let p = mat(&e, "P");
    let u = mat(&e, "U");
    let n_nodes = p.cols();
    assert_eq!(u.rows(), n_nodes);
    assert_eq!(u.cols(), 3);

    let omega = 2.0_f64.sqrt() * PI;
    let c_half = (omega * 0.5).cos();
    let mut max_err = 0.0f64;
    for i in 0..n_nodes {
        let x = p.get(0, i);
        let y = p.get(1, i);
        let exact = c_half * (PI * x).sin() * (PI * y).sin();
        max_err = max_err.max((u.get(i, 2) - exact).abs());
    }
    eprintln!("wave t=0.5 max_err: {max_err:.3e}");
    assert!(
        max_err <= 5e-3,
        "wave equation max abs error at t=0.5 is {max_err:.3e} > 5e-3"
    );
}

#[test]
fn set_initial_conditions_from_stationary_results_and_face_override() {
    // Solve stationary Poisson problem, then use its StationaryResults as u0 for heat decay.
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.2, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 1);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         r_stat = solvepde(m);\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 1);\n\
         setInitialConditions(m, r_stat);\n\
         res = solvepde(m, [0 0.05 0.1]);\n\
         u_stat = r_stat.NodalSolution;\n\
         u_t0 = res.NodalSolution(:, 1);\n\
         u_t2 = res.NodalSolution(:, 3);\n\
         diff0 = max(abs(u_t0 - u_stat));\n\
         diff2 = max(abs(u_t2 - u_stat));\n"
    ));
    // Stationary solution is an exact equilibrium of the same PDE with d=1, so u(t) == u_stat!
    assert!(scalar(&e, "diff0") <= 1e-14);
    assert!(
        scalar(&e, "diff2") <= 1e-5,
        "equilibrium drifted by {}",
        scalar(&e, "diff2")
    );

    // Per-face setInitialConditions on two-face geometry.
    let e2 = run(&format!(
        "{TWO_FACES}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         setInitialConditions(m, 1, 'Face', 1);\n\
         setInitialConditions(m, 3, 'Face', 2);\n\
         res = solvepde(m, [0 0.5 2.0]);\n\
         P = res.Mesh.Nodes;\n\
         u0 = res.NodalSolution(:, 1);\n\
         u_end = res.NodalSolution(:, 3);\n\
         left_err = max(abs(u0(P(1,:) < 0.99) - 1));\n\
         right_err = max(abs(u0(P(1,:) > 1.01) - 3));\n\
         spread = max(u_end) - min(u_end);\n\
         mean_u = mean(u_end);\n"
    ));
    // Face 1 interior starts at 1, Face 2 starts at 3; insulated domain relaxes to a uniform state.
    assert!(scalar(&e2, "left_err") <= 1e-14);
    assert!(scalar(&e2, "right_err") <= 1e-14);
    assert!(
        scalar(&e2, "spread") <= 0.03,
        "expected uniform relaxation, spread = {}",
        scalar(&e2, "spread")
    );
    let mean_u = scalar(&e2, "mean_u");
    assert!(
        (1.8..=2.2).contains(&mean_u),
        "expected relaxed mean near 2, got {mean_u}"
    );
}

#[test]
fn time_dependent_dirichlet_bc_receives_state_time() {
    // Spatially uniform solution u(x,y,t) = t satisfies d*u_t - c*lap(u) = f with d=1, c=1, f=1,
    // u0 = 0, and Dirichlet BC u(boundary, t) = state.time.
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 1);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', @(loc, state) state.time);\n\
         setInitialConditions(m, 0);\n\
         m.SolverOptions.RelativeTolerance = 1e-6;\n\
         m.SolverOptions.AbsoluteTolerance = 1e-9;\n\
         res = solvepde(m, [0 0.25 0.5 1.0]);\n\
         U = res.NodalSolution;\n\
         T = res.SolutionTimes;\n"
    ));
    let u = mat(&e, "U");
    let t = mat(&e, "T");
    let mut max_err = 0.0f64;
    for k in 0..t.cols() {
        let tk = t.get(0, k);
        for i in 0..u.rows() {
            max_err = max_err.max((u.get(i, k) - tk).abs());
        }
    }
    assert!(
        max_err <= 1e-5,
        "time-dependent Dirichlet BC u(x,y,t)=t max err {max_err:.3e} > 1e-5"
    );
}

#[test]
fn is_time_dependent_recomputed_on_override() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         td1 = double(m.IsTimeDependent);\n\
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 1);\n\
         td2 = double(m.IsTimeDependent);\n\
         specifyCoefficients(m, 'm', 1, 'd', 0, 'c', 1, 'a', 0, 'f', 0, 'Face', 1);\n\
         td3 = double(m.IsTimeDependent);\n\
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 1, 'Face', 1);\n\
         td4 = double(m.IsTimeDependent);\n"
    ));
    assert_eq!(scalar(&e, "td1"), 1.0);
    assert_eq!(scalar(&e, "td2"), 0.0);
    assert_eq!(scalar(&e, "td3"), 1.0);
    assert_eq!(scalar(&e, "td4"), 0.0);
}

#[test]
fn solver_options_and_results_object_metadata() {
    let mut e = Engine::default();
    let out = e
        .execute(&format!(
            "{UNIT_SQUARE}\
             m = createpde();\n\
             rt0 = m.SolverOptions.RelativeTolerance;\n\
             at0 = m.SolverOptions.AbsoluteTolerance;\n\
             m.SolverOptions.RelativeTolerance = 1e-5;\n\
             m.SolverOptions.AbsoluteTolerance = 1e-8;\n\
             rt1 = m.SolverOptions.RelativeTolerance;\n\
             at1 = m.SolverOptions.AbsoluteTolerance;\n\
             geometryFromEdges(m, dl);\n\
             generateMesh(m, 'Hmax', 0.3, 'GeometricOrder', 'linear');\n\
             specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
             applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
             setInitialConditions(m, 1);\n\
             res = solvepde(m, [0 0.1]);\n\
             eq_self = double(isequal(res, res));\n\
             s = get(res);\n\
             st_len = numel(s.SolutionTimes);\n\
             disp(res);\n"
        ))
        .unwrap();

    assert_eq!(scalar(&e, "rt0"), 1e-3);
    assert_eq!(scalar(&e, "at0"), 1e-6);
    assert_eq!(scalar(&e, "rt1"), 1e-5);
    assert_eq!(scalar(&e, "at1"), 1e-8);
    assert_eq!(scalar(&e, "eq_self"), 1.0);
    assert_eq!(scalar(&e, "st_len"), 2.0);
    let disp_text = out.output.join("\n");
    for prop in ["NodalSolution", "SolutionTimes", "Mesh"] {
        assert!(
            disp_text.contains(prop),
            "display missing property {prop}:\n{disp_text}"
        );
    }
}

#[test]
fn error_cases_time_dependent() {
    // tlist not increasing.
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); generateMesh(m, 'Hmax', 0.4); \
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0); \
         setInitialConditions(m, 0); solvepde(m, [0 0.2 0.1]);"
    ));
    assert!(
        err.to_lowercase().contains("increasing"),
        "unexpected err: {err}"
    );

    // tlist equal consecutive values.
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); generateMesh(m, 'Hmax', 0.4); \
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0); \
         setInitialConditions(m, 0); solvepde(m, [0 0]);"
    ));
    assert!(
        err.to_lowercase().contains("increasing"),
        "unexpected err: {err}"
    );

    // tlist with only 1 point.
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); generateMesh(m, 'Hmax', 0.4); \
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0); \
         setInitialConditions(m, 0); solvepde(m, [0]);"
    ));
    assert!(
        err.to_lowercase().contains("tlist"),
        "unexpected err: {err}"
    );

    // Missing initial condition.
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); generateMesh(m, 'Hmax', 0.4); \
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0); \
         solvepde(m, [0 0.1]);"
    ));
    assert!(
        err.to_lowercase().contains("initial condition"),
        "unexpected err: {err}"
    );

    // m = 0 and d = 0 with tlist.
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); generateMesh(m, 'Hmax', 0.4); \
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 1); \
         setInitialConditions(m, 0); solvepde(m, [0 0.1]);"
    ));
    assert!(
        err.to_lowercase().contains("stationary") || err.contains("tlist"),
        "unexpected err: {err}"
    );

    // Read-only TimeDependentResults property.
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); generateMesh(m, 'Hmax', 0.4); \
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0); \
         setInitialConditions(m, 0); r = solvepde(m, [0 0.1]); r.SolutionTimes = 0;"
    ));
    assert!(
        err.to_lowercase().contains("read-only"),
        "unexpected err: {err}"
    );
}

#[test]
fn time_dependent_c_coefficient_is_rejected() {
    let err = error(&format!(
        "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); generateMesh(m, 'Hmax', 0.4); \
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', @(loc,state) 1+100*state.time, 'a', 0, 'f', 0); \
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0); \
         setInitialConditions(m, 0); solvepde(m, [0 0.1]);"
    ));
    assert!(
        err.contains("time-dependent 'c' coefficient is not supported yet"),
        "unexpected err: {err}"
    );
}

#[test]
fn sqrt_time_dirichlet_at_tlist_start_solves() {
    // u = sqrt(state.time) is undefined for t < 0. The one-sided difference at tlist(1) = 0
    // must not evaluate the handle before the first sample.
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.5, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', @(loc, state) sqrt(state.time));\n\
         setInitialConditions(m, 0);\n\
         res = solvepde(m, [0 0.1 0.2]);\n\
         P = res.Mesh.Nodes;\n\
         U = res.NodalSolution;\n"
    ));
    let p = mat(&e, "P");
    let u = mat(&e, "U");
    assert_eq!(u.cols(), 3);
    let sqrt_end = 0.2_f64.sqrt();
    let mut n_bd = 0usize;
    let mut bd0 = 0.0_f64;
    let mut bd_end = 0.0_f64;
    for i in 0..u.rows() {
        let x = p.get(0, i);
        let y = p.get(1, i);
        let on_bd = x <= 1e-9 || x >= 1.0 - 1e-9 || y <= 1e-9 || y >= 1.0 - 1e-9;
        if !on_bd {
            assert!(u.get(i, 0).is_finite() && u.get(i, 2).is_finite());
            continue;
        }
        n_bd += 1;
        bd0 = bd0.max(u.get(i, 0).abs());
        bd_end = bd_end.max((u.get(i, 2) - sqrt_end).abs());
    }
    assert!(n_bd > 0, "expected boundary nodes");
    assert!(bd0 <= 1e-12, "boundary at t=0 is {bd0}");
    assert!(
        bd_end <= 1e-12,
        "boundary at t=0.2 differs from sqrt(0.2) by {bd_end}"
    );
}

#[test]
fn nodal_column1_holds_dirichlet_not_raw_u0() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.3, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0);\n\
         applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         setInitialConditions(m, 1);\n\
         res = solvepde(m, [0 0.05]);\n\
         P = res.Mesh.Nodes;\n\
         U = res.NodalSolution;\n"
    ));
    let p = mat(&e, "P");
    let u = mat(&e, "U");
    let mut n_bd = 0usize;
    let mut n_in = 0usize;
    let mut bd_max = 0.0_f64;
    let mut interior_err = 0.0_f64;
    for i in 0..u.rows() {
        let x = p.get(0, i);
        let y = p.get(1, i);
        let on_bd = x <= 1e-9 || x >= 1.0 - 1e-9 || y <= 1e-9 || y >= 1.0 - 1e-9;
        if on_bd {
            n_bd += 1;
            bd_max = bd_max.max(u.get(i, 0).abs());
        } else {
            n_in += 1;
            interior_err = interior_err.max((u.get(i, 0) - 1.0).abs());
        }
    }
    assert!(n_bd > 0 && n_in > 0, "bd={n_bd} interior={n_in}");
    assert!(
        bd_max <= 1e-14,
        "column 1 boundary nodes must be u_D(t0)=0, max abs {bd_max}"
    );
    assert!(
        interior_err <= 1e-14,
        "column 1 interior nodes must stay at u0=1, max abs err {interior_err}"
    );
}

#[test]
fn mass_coefficient_must_be_nonzero_on_every_face() {
    let err = error(&format!(
        "{TWO_FACES} m = createpde(); geometryFromEdges(m, dl); generateMesh(m, 'Hmax', 0.4); \
         specifyCoefficients(m, 'm', 0, 'd', 1, 'c', 1, 'a', 0, 'f', 0, 'Face', 1); \
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 0, 'Face', 2); \
         setInitialConditions(m, 0); solvepde(m, [0 0.1]);"
    ));
    assert!(
        err.contains("must be nonzero on every face"),
        "unexpected err: {err}"
    );
    assert!(
        !err.to_lowercase().contains("minimum"),
        "fell through to ode15s instead of the face check: {err}"
    );

    let err_m = error(&format!(
        "{TWO_FACES} m = createpde(); geometryFromEdges(m, dl); generateMesh(m, 'Hmax', 0.4); \
         specifyCoefficients(m, 'm', 1, 'd', 0, 'c', 1, 'a', 0, 'f', 0, 'Face', 1); \
         specifyCoefficients(m, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 0, 'Face', 2); \
         setInitialConditions(m, 0, 0); solvepde(m, [0 0.1]);"
    ));
    assert!(
        err_m.contains("must be nonzero on every face"),
        "unexpected err: {err_m}"
    );
}

/// Unit square heat problem d = 1, c = 1, a = 0, u0 = 0 with the given coefficient/BC lines;
/// returns NodalSolution (nodes x Nt) and node coordinates.
fn pulse_solve(coeffs: &str, bcs: &str, tlist: &str) -> (Matrix, Matrix) {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m = createpde(); geometryFromEdges(m, dl);\n\
         generateMesh(m, 'Hmax', 0.25, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(m, 'm', 0, 'd', 1, {coeffs});\n\
         {bcs}\n\
         setInitialConditions(m, 0);\n\
         res = solvepde(m, {tlist});\n\
         U = res.NodalSolution;\n\
         P = res.Mesh.Nodes;\n"
    ));
    (mat(&e, "U"), mat(&e, "P"))
}

fn col_max_abs(u: &Matrix, k: usize, interior_of: Option<&Matrix>) -> f64 {
    (0..u.rows())
        .filter(|&i| {
            interior_of.is_none_or(|p| {
                let (x, y) = (p.get(0, i), p.get(1, i));
                x > 1e-9 && x < 1.0 - 1e-9 && y > 1e-9 && y < 1.0 - 1e-9
            })
        })
        .map(|i| u.get(i, k).abs())
        .fold(0.0, f64::max)
}

const PULSE: &str = "double(s.time > 0.2 & s.time < 0.4) + 0*l.x";

#[test]
fn time_pulse_in_f_g_and_dirichlet_u_is_seen() {
    // A pulse on (0.2, 0.4) is zero at t0, at the midpoint 0.5, and at t_end = 1: a 3-point
    // probe misses it. Every step must evaluate the handle with state.time.
    let tlist = "0:0.1:1";
    let zero_bc = "applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);";
    let (u_f, _) = pulse_solve(
        &format!("'c', 1, 'a', 0, 'f', @(l,s) {PULSE}"),
        zero_bc,
        tlist,
    );
    let (u_g, _) = pulse_solve(
        "'c', 1, 'a', 0, 'f', 0",
        &format!(
            "applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:3, 'u', 0); \
             applyBoundaryCondition(m, 'neumann', 'Edge', 4, 'g', @(l,s) {PULSE});"
        ),
        tlist,
    );
    let (u_d, p_d) = pulse_solve(
        "'c', 1, 'a', 0, 'f', 0",
        &format!("applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', @(l,s) {PULSE});"),
        tlist,
    );
    for (label, u, p) in [
        ("f", &u_f, None),
        ("g", &u_g, None),
        ("Dirichlet u", &u_d, Some(&p_d)),
    ] {
        assert_eq!(u.cols(), 11);
        // t = 0.1: the step that crosses the jump at t = 0.2 may also cover t = 0.2 itself
        // (ode15s interpolates outputs), so the zero check uses the previous output time.
        let before = col_max_abs(u, 1, p);
        let at_03 = col_max_abs(u, 3, p);
        let at_05 = col_max_abs(u, 5, p);
        eprintln!("pulse {label}: t=0.1 {before:.3e}, t=0.3 {at_03:.3e}, t=0.5 {at_05:.3e}");
        assert!(
            before <= 1e-4,
            "pulse {label}: nonzero before the pulse ({before:.3e})"
        );
        assert!(
            at_03 > 1e-3,
            "pulse {label}: NodalSolution at t=0.3 is {at_03:.3e}"
        );
        assert!(
            at_05 > 1e-4,
            "pulse {label}: NodalSolution at t=0.5 is {at_05:.3e}"
        );
    }
}

#[test]
fn time_dependent_f_missed_by_single_point_probe_is_seen() {
    // The probe point used to be one quadrature point with x > 0.5, where this f is 0.
    let zero_bc = "applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);";
    let (u, _) = pulse_solve(
        "'c', 1, 'a', 0, 'f', @(l,s) max(0, s.time).*(l.x < 0.5)",
        zero_bc,
        "[0 0.5 1]",
    );
    let (u_mirror, _) = pulse_solve(
        "'c', 1, 'a', 0, 'f', @(l,s) max(0, s.time).*(l.x > 0.5)",
        zero_bc,
        "[0 0.5 1]",
    );
    let (v, vm) = (col_max_abs(&u, 2, None), col_max_abs(&u_mirror, 2, None));
    eprintln!("spatial f: x<0.5 {v:.3e}, x>0.5 {vm:.3e}");
    assert!(v > 1e-2, "f on x<0.5 gave {v:.3e}");
    assert!(
        (v - vm).abs() <= 0.05 * vm,
        "mirror images differ: {v:.3e} vs {vm:.3e}"
    );
}

#[test]
fn time_pulse_in_m_d_a_c_q_is_rejected() {
    let tlist = "0:0.1:1";
    let zero_bc = "applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:4, 'u', 0);";
    let src = |coeffs: &str, bcs: &str| {
        format!(
            "{UNIT_SQUARE} m = createpde(); geometryFromEdges(m, dl); \
             generateMesh(m, 'Hmax', 0.25, 'GeometricOrder', 'linear'); \
             specifyCoefficients(m, 'm', 0, {coeffs}); {bcs} \
             setInitialConditions(m, 0); res = solvepde(m, {tlist});"
        )
    };
    let cases = [
        (
            format!("'d', 1, 'c', @(l,s) 1 + 50*({PULSE}), 'a', 0, 'f', 1"),
            zero_bc.to_string(),
            "time-dependent 'c' coefficient",
        ),
        (
            format!("'d', 1, 'c', 1, 'a', @(l,s) 50*({PULSE}), 'f', 1"),
            zero_bc.to_string(),
            "time-dependent 'a' coefficient",
        ),
        (
            format!("'d', @(l,s) 1 + 50*({PULSE}), 'c', 1, 'a', 0, 'f', 1"),
            zero_bc.to_string(),
            "time-dependent 'm' or 'd'",
        ),
        (
            "'d', 1, 'c', 1, 'a', 0, 'f', 1".to_string(),
            format!(
                "applyBoundaryCondition(m, 'dirichlet', 'Edge', 1:3, 'u', 0); \
                 applyBoundaryCondition(m, 'neumann', 'Edge', 4, 'q', @(l,s) 5*({PULSE}));"
            ),
            "time-dependent Neumann 'q'",
        ),
    ];
    for (coeffs, bcs, want) in cases {
        let err = error(&src(&coeffs, &bcs));
        assert!(
            err.contains(want),
            "{coeffs} / {bcs}: unexpected err: {err}"
        );
    }
    // A state-taking handle whose value does not change in time still solves.
    let (u, _) = pulse_solve(
        "'c', @(l,s) 1 + 0*l.x, 'a', @(l,s) 0*l.x, 'f', 1",
        zero_bc,
        "[0 0.1 0.2]",
    );
    assert!(col_max_abs(&u, 2, None) > 1e-3);
}
