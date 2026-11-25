//! Independent cvxpy/OSQP fixtures from tool/refs/mpc/qp.py; no MATLAB runs.
use matrust::{Array, Engine, Matrix};
use serde_json::Value as Json;
use std::time::{Duration, Instant};

fn fixtures() -> Json {
    serde_json::from_str(include_str!("fixtures/mpc/qp.json")).unwrap()
}
fn vec(j: &Json) -> Vec<f64> {
    j.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect()
}
fn matrix(j: &Json, n: usize) -> Matrix {
    let rows = j.as_array().unwrap();
    Matrix::from_column_vec(
        rows.len(),
        n,
        (0..n)
            .flat_map(|c| rows.iter().map(move |r| r[c].as_f64().unwrap()))
            .collect(),
    )
    .unwrap()
}
fn setup(j: &Json) -> Engine {
    let mut e = Engine::default();
    let n = j["f"].as_array().unwrap().len();
    for (name, key) in [("H", "H"), ("A", "A"), ("E", "Aeq"), ("Linv", "Linv")] {
        e.insert_array(name, Array::Numeric(matrix(&j[key], n)))
            .unwrap();
    }
    for (name, key) in [("f", "f"), ("b", "b"), ("d", "beq")] {
        let v = vec(&j[key]);
        e.insert_array(
            name,
            Array::Numeric(Matrix::from_column_vec(v.len(), 1, v).unwrap()),
        )
        .unwrap();
    }
    run(
        &mut e,
        "o=mpcActiveSetOptions; o.ConstraintTolerance=1e-9; o.MaxIterations=1000; w=false(size(b));",
    );
    e
}
fn run(e: &mut Engine, source: &str) {
    e.execute(source)
        .unwrap_or_else(|err| panic!("{source}\n{err}"));
}
fn values(e: &Engine, name: &str) -> Vec<f64> {
    match e.array(name) {
        Some(Array::Numeric(m)) => m.data().to_vec(),
        Some(Array::Typed(t)) => t.to_f64_vec().unwrap(),
        Some(Array::Logical(l)) => l.data().iter().map(|&x| f64::from(x)).collect(),
        v => panic!("unexpected {name}: {v:?}"),
    }
}
fn scalar(e: &Engine, name: &str) -> f64 {
    values(e, name)[0]
}
fn solve(e: &mut Engine) {
    run(
        e,
        "[x,flag,active,lambda]=mpcActiveSetSolver(H,f,A,b,E,d,w,o); zi=lambda.ineqlin; ye=lambda.eqlin;",
    );
}
fn max_error(a: &[f64], b: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0., f64::max)
}
fn case<'a>(fixture: &'a Json, name: &str) -> &'a Json {
    fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["name"] == name)
        .unwrap()
}
fn validate(e: &mut Engine, j: &Json) -> (f64, f64) {
    assert!(
        scalar(e, "flag") > 0.,
        "{} flag={}",
        j["name"],
        scalar(e, "flag")
    );
    let error = max_error(&values(e, "x"), &vec(&j["x"]));
    assert!(error < 1e-6, "{} x error {error:e}", j["name"]);
    run(
        e,
        "obj=0.5*x'*H*x+f'*x; station=H*x+f+A'*zi+E'*ye; slack=A*x-b; eqres=E*x-d; comp=zi.*slack;",
    );
    let expected = j["objective"].as_f64().unwrap();
    let relative = (scalar(e, "obj") - expected).abs() / expected.abs().max(1.);
    assert!(
        relative < 1e-8,
        "{} objective relative error {relative:e}",
        j["name"]
    );
    assert!(
        values(e, "station").iter().all(|v| v.abs() < 1e-7),
        "{} stationarity",
        j["name"]
    );
    assert!(
        values(e, "slack").iter().all(|v| *v <= 2e-9),
        "{} primal feasibility",
        j["name"]
    );
    assert!(values(e, "eqres").iter().all(|v| v.abs() < 2e-9));
    assert!(values(e, "zi").iter().all(|v| *v >= -1e-9));
    assert!(values(e, "comp").iter().all(|v| v.abs() < 1e-7));
    assert_eq!(values(e, "zi").len(), vec(&j["b"]).len());
    assert_eq!(values(e, "ye").len(), vec(&j["beq"]).len());
    (error, relative)
}

#[test]
fn reference_qps_and_kkt() {
    let fixture = fixtures();
    let mut worst_x: f64 = 0.;
    let mut worst_objective: f64 = 0.;
    for j in fixture["cases"].as_array().unwrap() {
        let mut e = setup(j);
        solve(&mut e);
        let (x, obj) = validate(&mut e, j);
        worst_x = worst_x.max(x);
        worst_objective = worst_objective.max(obj);
        println!(
            "{}: iterations={}, max x error={x:e}, objective error={obj:e}",
            j["name"],
            scalar(&e, "flag")
        );
    }
    println!("MPC QP reference maxima: x={worst_x:e}, relative objective={worst_objective:e}");
}
#[test]
fn unconstrained_matches_linear_solve() {
    let fixture = fixtures();
    let j = case(&fixture, "unconstrained");
    let mut e = setup(j);
    solve(&mut e);
    run(&mut e, "closed=-(H\\f);");
    assert!(max_error(&values(&e, "x"), &values(&e, "closed")) < 1e-12);
    assert!(max_error(&values(&e, "x"), &vec(&j["closed_form"])) < 1e-12);
    assert_eq!(scalar(&e, "flag"), 1.);
}
#[test]
fn optimal_active_set_warm_starts_in_one_iteration() {
    let fixture = fixtures();
    for j in fixture["cases"].as_array().unwrap() {
        let mut e = setup(j);
        solve(&mut e);
        run(&mut e, "previous=x; w=active; o.MaxIterations=1;");
        solve(&mut e);
        assert_eq!(scalar(&e, "flag"), 1., "{}", j["name"]);
        assert!(max_error(&values(&e, "previous"), &values(&e, "x")) < 1e-8);
        validate(&mut e, j);
    }
}
#[test]
fn inverse_lower_cholesky_input() {
    let fixture = fixtures();
    for j in fixture["cases"].as_array().unwrap() {
        let mut e = setup(j);
        run(
            &mut e,
            "o.UseHessianAsInput=false; [x,flag,active,lambda]=mpcActiveSetSolver(Linv,f,A,b,E,d,w,o); zi=lambda.ineqlin; ye=lambda.eqlin;",
        );
        validate(&mut e, j);
    }
}
#[test]
fn mpc_horizon_ten_obeys_move_and_rate_limits() {
    let fixture = fixtures();
    let j = case(&fixture, "mpc_horizon_10");
    let mut e = setup(j);
    solve(&mut e);
    validate(&mut e, j);
    let u = values(&e, "x");
    let mut last = 0.;
    for u in u {
        assert!(u.abs() <= 0.7 + 1e-9);
        assert!((u - last).abs() <= 0.12 + 1e-9);
        last = u;
    }
    assert!((values(&e, "x")[0] + 0.12).abs() < 1e-8);
}
#[test]
fn bounds_and_duplicate_constraints() {
    let fixture = fixtures();
    let j = case(&fixture, "box");
    let mut e = setup(j);
    solve(&mut e);
    assert!(max_error(&values(&e, "x"), &[1., -1., 1., -1.]) < 1e-12);
    let j = case(&fixture, "duplicates");
    let mut e = setup(j);
    // Even redundant warm guesses retain only an independent working basis.
    run(&mut e, "w=true(size(b));");
    solve(&mut e);
    validate(&mut e, j);
    assert!(values(&e, "active").iter().sum::<f64>() <= 2.);
}
#[test]
fn infeasibility_including_zero_and_dependent_rows() {
    for (a, b, eq, d) in [
        ("[1;-1]", "[0;-1]", "zeros(0,1)", "zeros(0,1)"),
        ("0", "-1", "zeros(0,1)", "zeros(0,1)"),
        ("1", "0", "1", "1"),
    ] {
        let mut e = Engine::default();
        run(
            &mut e,
            &format!(
                "o=mpcActiveSetOptions; A={a}; b={b}; [x,flag,active,lambda]=mpcActiveSetSolver(1,0,A,b,{eq},{d},false(size(b)),o);"
            ),
        );
        assert_eq!(scalar(&e, "flag"), -1.);
        assert_eq!(values(&e, "x").len(), 1);
    }
    let mut e = Engine::default();
    run(
        &mut e,
        "o=mpcActiveSetOptions; [x,flag]=mpcActiveSetSolver(eye(2),[0;0],zeros(0,2),zeros(0,1),[1 0;2 0],[0;1],false(0,1),o);",
    );
    assert_eq!(scalar(&e, "flag"), -1.);
}
#[test]
fn iteration_limit_retains_an_iterate() {
    let fixture = fixtures();
    let mut e = setup(case(&fixture, "box"));
    run(&mut e, "o.MaxIterations=1;");
    solve(&mut e);
    assert_eq!(scalar(&e, "flag"), 0.);
    assert_eq!(values(&e, "x").len(), 4);
    assert!(values(&e, "x").iter().all(|v| v.is_finite()));
}
#[test]
fn changed_problem_and_bad_warm_guess_recover() {
    let fixture = fixtures();
    let j = case(&fixture, "box");
    let mut e = setup(j);
    solve(&mut e);
    run(&mut e, "w=active; f=-f;");
    solve(&mut e);
    assert!(scalar(&e, "flag") > 0.);
    assert!(max_error(&values(&e, "x"), &[-1., 1., -1., 1.]) < 1e-12);
    run(&mut e, "w=true(size(b));");
    solve(&mut e);
    assert!(scalar(&e, "flag") > 0.);
    assert!(max_error(&values(&e, "x"), &[-1., 1., -1., 1.]) < 1e-12);
}
#[test]
fn options_defaults_single_and_integrity_checks() {
    let mut e = Engine::default();
    run(
        &mut e,
        "o=mpcActiveSetOptions; it=o.MaxIterations; tol=o.ConstraintTolerance; ht=o.UseHessianAsInput; ic=o.IntegrityChecks;",
    );
    assert_eq!(scalar(&e, "it"), 200.);
    assert_eq!(scalar(&e, "tol"), 1e-6);
    assert_eq!(scalar(&e, "ht"), 1.);
    assert_eq!(scalar(&e, "ic"), 1.);
    run(
        &mut e,
        "o=mpcActiveSetOptions('single'); o.IntegrityChecks=false; [x,flag,active,lambda]=mpcActiveSetSolver(single(eye(2)),single([-2;-4]),single(eye(2)),single([1;2]),single(zeros(0,2)),single(zeros(0,1)),false(2,1),o); zi=lambda.ineqlin;",
    );
    assert!(scalar(&e, "flag") > 0.);
    assert!(max_error(&values(&e, "x"), &[1., 2.]) < 1e-6);
    assert!(matches!(e.array("x"), Some(Array::Typed(_))));
    assert!(matches!(e.array("zi"), Some(Array::Typed(_))));
    assert!(max_error(&values(&e, "zi"), &[1., 2.]) < 1e-6);
}
#[test]
fn rejects_non_pd_hessians_and_malformed_inputs() {
    for h in ["0", "-1", "[1 2;2 1]", "[1 0;1 1]", "[NaN 0;0 1]"] {
        let mut e = Engine::default();
        let err=e.execute(&format!("o=mpcActiveSetOptions; H={h}; n=size(H,1); x=mpcActiveSetSolver(H,zeros(n,1),zeros(0,n),zeros(0,1),zeros(0,n),zeros(0,1),false(0,1),o);")).unwrap_err();
        assert!(
            err.contains("positive definite") || err.contains("finite"),
            "{err}"
        );
    }
    for code in [
        "o.MaxIterations=0;",
        "o.MaxIterations=1.5;",
        "o.ConstraintTolerance=NaN;",
        "o.ConstraintTolerance=-1;",
        "o.Unknown=1;",
        "o.DataType='bad';",
        "o.UseHessianAsInput=0;",
        "o.IntegrityChecks=1;",
    ] {
        let mut e = Engine::default();
        assert!(
            e.execute(&format!(
                "o=mpcActiveSetOptions; {code} x=mpcActiveSetSolver(1,0,[],[],[],[],false(0,1),o);"
            ))
            .is_err(),
            "{code}"
        );
    }
    for call in [
        "mpcActiveSetOptions('int32')",
        "mpcActiveSetOptions(1)",
        "mpcActiveSetSolver(1,0,[],[],[],[],[],o)",
        "mpcActiveSetSolver(eye(2),[0 0],[],[],[],[],false(0,1),o)",
        "mpcActiveSetSolver(1,0,[1 1],0,[],[],false(1,1),o)",
        "mpcActiveSetSolver(1,0,1,0,[],[],false(0,1),o)",
        "mpcActiveSetSolver(eye(201),zeros(201,1),[],[],[],[],false(0,1),o)",
    ] {
        let mut e = Engine::default();
        assert!(
            e.execute(&format!("o=mpcActiveSetOptions; x={call};"))
                .is_err(),
            "{call}"
        );
    }
}
#[test]
fn empty_constraints_and_single_output_dispatch() {
    let mut e = Engine::default();
    run(
        &mut e,
        "o=mpcActiveSetOptions(); x=mpcActiveSetSolver(2,-4,[],[],[],[],false(0,1),o); y=feval('mpcActiveSetSolver',2,-4,[],[],[],[],false(0,1),o);",
    );
    assert!((scalar(&e, "x") - 2.).abs() < 1e-14);
    assert!((scalar(&e, "y") - 2.).abs() < 1e-14);
}
#[test]
fn cancellation_and_host_workspace_limit() {
    let fixture = fixtures();
    let j = case(&fixture, "random_60_180");
    let mut e = setup(j);
    let mut limits = e.limits_active();
    limits.memory_bytes = 50_000;
    e.limits_set(limits);
    let err = e
        .execute("[x,flag]=mpcActiveSetSolver(H,f,A,b,E,d,w,o);")
        .unwrap_err();
    assert!(err.contains("memory"), "{err}");
    // Cancellation is checked within whitening and every active-set step, as
    // well as by the engine at entry. A pre-cancelled run must not publish x.
    let mut e = setup(j);
    e.cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        e.execute("[x,flag]=mpcActiveSetSolver(H,f,A,b,E,d,w,o);")
            .is_err()
    );
    assert!(e.array("x").is_none());
}
#[test]
fn speed_n60_m180() {
    let fixture = fixtures();
    let j = case(&fixture, "random_60_180");
    let mut e = setup(j);
    let start = Instant::now();
    solve(&mut e);
    let elapsed = start.elapsed();
    println!(
        "MPC dense QP n=60 m=180: {elapsed:?}; RAYON_NUM_THREADS={:?}; iterations={}",
        std::env::var("RAYON_NUM_THREADS"),
        scalar(&e, "flag")
    );
    validate(&mut e, j);
    assert!(
        elapsed < Duration::from_millis(500),
        "dense QP took {elapsed:?}"
    );
}

#[test]
fn numerical_overflow_reports_unreliable_and_a_finite_fallback() {
    let mut e = Engine::default();
    run(
        &mut e,
        "o=mpcActiveSetOptions; [x,flag]=mpcActiveSetSolver(1e-300,1e300,[],[],[],[],false(0,1),o);",
    );
    assert_eq!(scalar(&e, "flag"), -2.);
    assert!(values(&e, "x").iter().all(|v| v.is_finite()));
}

#[test]
fn single_output_keeps_single_inputs_and_class_validation() {
    let mut e = Engine::default();
    run(
        &mut e,
        "o=mpcActiveSetOptions('single'); x=mpcActiveSetSolver(single(2),single(-4),single([]),single([]),single([]),single([]),false(0,1),o);",
    );
    assert_eq!(scalar(&e, "x"), 2.);
    assert!(matches!(e.array("x"), Some(Array::Typed(_))));
    let mut e = Engine::default();
    assert!(e.execute("o=mpcActiveSetOptions; x=mpcActiveSetSolver(single(2),single(-4),[],[],[],[],false(0,1),o);").is_err());
}

#[test]
fn row_scaling_preserves_solution_and_multiplier_units() {
    let mut e = Engine::default();
    run(
        &mut e,
        "o=mpcActiveSetOptions; o.ConstraintTolerance=1e-12; [x,flag,active,lambda]=mpcActiveSetSolver(eye(2),[-2;-3],[1e-8 0;0 1e8],[1e-8;2e8],zeros(0,2),zeros(0,1),false(2,1),o); zi=lambda.ineqlin;",
    );
    assert!(scalar(&e, "flag") > 0.);
    assert!(max_error(&values(&e, "x"), &[1., 2.]) < 1e-12);
    assert!((values(&e, "zi")[0] / 1e8 - 1.).abs() < 1e-12);
    assert!((values(&e, "zi")[1] / 1e-8 - 1.).abs() < 1e-12);
}
