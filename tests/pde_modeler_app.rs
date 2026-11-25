//! Integration tests for PDE Modeler scripts (elliptic and parabolic samples):
//! batch-mode script replay vs in-process run, physical properties, and DOF bounds.
use matrust::{Array, Engine, Matrix, engine::pde_app_model::PdeAppModel};
use std::process::Command;
use std::time::Instant;

fn matrust_bin() -> &'static str {
    env!("CARGO_BIN_EXE_matrust")
}

fn run_batch_and_load_u(script: &str) -> Matrix {
    let dir = tempfile::tempdir().expect("tempdir");
    let script_path = dir.path().join("pde_sample.m");
    let mat_path = dir.path().join("pde_out.mat");

    let mut full = script.to_owned();
    full.push_str(&format!(
        "\nU_batch = results.NodalSolution;\nsave('{}', 'U_batch');\n",
        mat_path.display().to_string().replace('\'', "''")
    ));
    std::fs::write(&script_path, full).expect("write temp .m");

    let batch_cmd = format!(
        "run('{}')",
        script_path.display().to_string().replace('\'', "''")
    );
    let output = Command::new(matrust_bin())
        .args(["-batch", &batch_cmd])
        .output()
        .expect("spawn matrust -batch");
    assert!(
        output.status.success(),
        "matrust -batch failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mut loader = Engine::default();
    loader
        .execute(&format!(
            "S = load('{}'); U = S.U_batch;",
            mat_path.display().to_string().replace('\'', "''")
        ))
        .expect("load temp .mat");
    let Array::Numeric(u) = loader.array("U").expect("U in loader") else {
        panic!("expected numeric matrix U");
    };
    u
}

fn max_abs_diff(a: &Matrix, b: &Matrix) -> f64 {
    assert_eq!(a.rows(), b.rows(), "row count mismatch");
    assert_eq!(a.cols(), b.cols(), "col count mismatch");
    a.data()
        .iter()
        .zip(b.data().iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0_f64, f64::max)
}

#[test]
fn pde_modeler_app_elliptic_plate_with_hole() {
    let model = PdeAppModel::plate_with_hole();
    let script = model.script().expect("valid elliptic sample script");

    let t0 = Instant::now();
    let mut engine = Engine::default();
    engine.execute(&script).expect("in-process elliptic solve");
    let solve_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // Verify decsg edge numbering: edges 1..4 outer square, 5..8 circular hole.
    engine
        .execute(
            "dl = decsg(pdem_gd, pdem_sf, pdem_ns);\n\
             U = results.NodalSolution;\n\
             P = results.Mesh.Nodes;\n\
             outer_nodes = findNodes(results.Mesh, 'region', 'Edge', 1:4);\n\
             hole_nodes = findNodes(results.Mesh, 'region', 'Edge', 5:8);\n",
        )
        .expect("extract elliptic results");

    let dl = engine.variables["dl"].clone();
    assert_eq!(dl.cols(), 8, "plate with hole should have 8 edges");
    for j in 0..4 {
        assert_eq!(
            dl.get(0, j),
            2.0,
            "edge {} should be outer line segment (type 2), got {}",
            j + 1,
            dl.get(0, j)
        );
    }
    for j in 4..8 {
        assert_eq!(
            dl.get(0, j),
            1.0,
            "edge {} should be hole circular arc (type 1), got {}",
            j + 1,
            dl.get(0, j)
        );
    }

    let u = engine.variables["U"].clone();
    let p = engine.variables["P"].clone();
    let outer_nodes = engine.variables["outer_nodes"].clone();
    let hole_nodes = engine.variables["hole_nodes"].clone();

    let n_nodes = p.cols();
    assert_eq!(u.rows(), n_nodes);
    assert_eq!(u.cols(), 1);
    assert!(n_nodes < 4096, "DOFs {n_nodes} must stay under 4096");

    let max_u = u.data().iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(max_u > 0.0, "max u must be positive, got {max_u}");

    let max_outer = outer_nodes
        .data()
        .iter()
        .map(|&idx| u.get(idx as usize - 1, 0).abs())
        .fold(0.0_f64, f64::max);
    assert!(
        max_outer <= 1e-12,
        "u on outer Dirichlet edges should be <= 1e-12, got {max_outer:.3e}"
    );

    let max_hole = hole_nodes
        .data()
        .iter()
        .map(|&idx| u.get(idx as usize - 1, 0).abs())
        .fold(0.0_f64, f64::max);
    assert!(
        max_hole > 1e-3,
        "hole Neumann boundary should have nonzero u, got {max_hole:.3e}"
    );

    // Compare with -batch run via temp .m and temp .mat.
    let u_batch = run_batch_and_load_u(&script);
    let diff = max_abs_diff(&u, &u_batch);
    assert!(
        diff <= 1e-10,
        "elliptic -batch vs in-process max abs diff {diff:.3e} > 1e-10"
    );

    eprintln!(
        "pde_modeler_app elliptic: nodes={n_nodes}, max_u={max_u:.6}, max_hole={max_hole:.6}, solve_ms={solve_ms:.1}"
    );
}

#[test]
fn pde_modeler_app_parabolic_plate_heat() {
    let model = PdeAppModel::plate_heat();
    let script = model.script().expect("valid parabolic sample script");

    let t0 = Instant::now();
    let mut engine = Engine::default();
    engine.execute(&script).expect("in-process parabolic solve");
    let solve_ms = t0.elapsed().as_secs_f64() * 1000.0;

    engine
        .execute("U = results.NodalSolution; P = results.Mesh.Nodes;")
        .expect("extract parabolic results");

    let u = engine.variables["U"].clone();
    let p = engine.variables["P"].clone();
    let n_nodes = p.cols();

    assert!(n_nodes < 4096, "parabolic DOFs {n_nodes} must be < 4096");
    assert_eq!(u.rows(), n_nodes);
    assert_eq!(u.cols(), 21, "parabolic sample should have 21 time columns");

    // u(:, 1) == 0 (initial condition u0 = 0).
    let max_t0 = (0..n_nodes)
        .map(|i| u.get(i, 0).abs())
        .fold(0.0_f64, f64::max);
    assert!(max_t0 <= 1e-12, "u(:,1) should be 0, got {max_t0:.3e}");

    // Find the centre-right node closest to (0.7, 0.0) and verify u increases in time.
    let mut best_node = 0usize;
    let mut best_dist = f64::INFINITY;
    for i in 0..n_nodes {
        let d = (p.get(0, i) - 0.7).hypot(p.get(1, i));
        if d < best_dist {
            best_dist = d;
            best_node = i;
        }
    }
    for k in 1..21 {
        let prev = u.get(best_node, k - 1);
        let curr = u.get(best_node, k);
        assert!(
            curr > prev,
            "centre-right node u should increase at step {k}: {prev} -> {curr}"
        );
    }

    // Compare with -batch run via temp .m and temp .mat.
    let u_batch = run_batch_and_load_u(&script);
    let diff = max_abs_diff(&u, &u_batch);
    assert!(
        diff <= 1e-10,
        "parabolic -batch vs in-process max abs diff {diff:.3e} > 1e-10"
    );

    let max_u_final = (0..n_nodes)
        .map(|i| u.get(i, 20))
        .fold(f64::NEG_INFINITY, f64::max);
    eprintln!(
        "pde_modeler_app parabolic: nodes={n_nodes}, max_u_t1={max_u_final:.6}, solve_ms={solve_ms:.1}"
    );
}
