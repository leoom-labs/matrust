//! fminimax, fgoalattain, patternsearch, ga, particleswarm and simulannealbnd.
//!
//! Expected values are optima derived by hand in the comments next to each
//! test (KKT conditions or symmetry), never MATLAB output. The stochastic
//! solvers are checked for reaching the known global minimum of Rastrigin's
//! function (0 at the origin), for repeatability under rng(seed), for bounds
//! and for their stopping rules; their iterates are implementation specific.
use matrust::{Array, Engine};
use std::time::{Duration, Instant};

fn run(e: &mut Engine, code: &str) -> Vec<String> {
    match e.execute(code) {
        Ok(out) => out.output,
        Err(error) => panic!("{code}\n{error}"),
    }
}
fn fails(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(_) => panic!("{code} should fail"),
        Err(error) => error.to_string(),
    }
}
fn values(e: &Engine, name: &str) -> Vec<f64> {
    if let Some(m) = e.variables.get(name) {
        return m.data().to_vec();
    }
    match e.array(name) {
        Some(Array::Numeric(m)) => m.data().to_vec(),
        Some(Array::Logical(m)) => m.data().iter().map(|&b| f64::from(b)).collect(),
        _ => panic!("missing numeric {name}"),
    }
}
fn scalar(e: &Engine, name: &str) -> f64 {
    let v = values(e, name);
    assert_eq!(v.len(), 1, "{name} is not a scalar: {v:?}");
    v[0]
}
fn text(e: &Engine, name: &str) -> String {
    match e.array(name) {
        Some(Array::Character(c)) => c.to_utf8(&e.cancellation()).unwrap(),
        _ => panic!("missing text {name}"),
    }
}
/// Values of an expression such as a struct field, through a temporary.
fn fvalues(e: &mut Engine, expr: &str) -> Vec<f64> {
    run(e, &format!("field_value__ = {expr};"));
    values(e, "field_value__")
}
fn fscalar(e: &mut Engine, expr: &str) -> f64 {
    let v = fvalues(e, expr);
    assert_eq!(v.len(), 1, "{expr} is not a scalar: {v:?}");
    v[0]
}
fn ftext(e: &mut Engine, expr: &str) -> String {
    run(e, &format!("field_value__ = {expr};"));
    text(e, "field_value__")
}
fn close(actual: &[f64], expected: &[f64], tol: f64, what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}: {actual:?}");
    for (a, b) in actual.iter().zip(expected) {
        assert!((a - b).abs() <= tol, "{what}: {actual:?} vs {expected:?}");
    }
}

/// Engine with Rastrigin's function as `ras` and a bound-checking copy as a
/// function file: bounded_ras errors if it is ever called outside [-5.12, 5.12].
fn setup() -> (Engine, tempfile::TempDir) {
    let mut e = Engine::default();
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    std::fs::write(
        dir.path().join("bounded_ras.m"),
        "function f = bounded_ras(x)\nif any(abs(x(:)) > 5.12)\nerror('bounds violated');\nend\nf = 20 + sum(x(:).^2 - 10*cos(2*pi*x(:)));\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("slow_global.m"),
        "function f = slow_global(x)\nk = 0;\nwhile k < 1000000\nf = sum(sort(x(:)).^2) + rand(); k = k + 1;\nend\nend\n",
    )
    .unwrap();
    e.set_function_directory(dir.path()).unwrap();
    run(
        &mut e,
        "ras = @(x) 20 + sum(x(:).^2 - 10*cos(2*pi*x(:))); lb = [-5.12 -5.12]; ub = [5.12 5.12];",
    );
    (e, dir)
}

#[test]
fn fminimax_documentation_style_five_objectives() {
    // F1 = 2x1^2+x2^2-48x1-40x2+304, F2 = -x1^2-3x2^2, F3 = x1+3x2-18,
    // F4 = -x1-x2, F5 = x1+x2-8. At x = (4,4): F = (0,-64,-2,-8,0), so F1 and
    // F5 are active with max 0. grad F1 = (4x1-48, 2x2-40) = (-32,-32),
    // grad F5 = (1,1); 1/33*gradF1 + 32/33*gradF5 = 0 with nonnegative weights
    // summing to 1, the first-order minimax condition (F1 convex, F5 linear).
    let (mut e, _dir) = setup();
    run(
        &mut e,
        "f = @(x) [2*x(1)^2+x(2)^2-48*x(1)-40*x(2)+304; -x(1)^2-3*x(2)^2; x(1)+3*x(2)-18; -x(1)-x(2); x(1)+x(2)-8];\
         [x,fval,maxfval,flag,out] = fminimax(f,[0.1 0.1],[],[],[],[],[],[],[],optimoptions('fminimax','Display','off'));",
    );
    close(&values(&e, "x"), &[4., 4.], 1e-4, "x");
    close(&values(&e, "fval"), &[0., -64., -2., -8., 0.], 1e-3, "fval");
    assert!(scalar(&e, "maxfval").abs() < 1e-4);
    assert!(scalar(&e, "flag") > 0.);
    assert!(fscalar(&mut e, "out.funcCount") > 1.);
    assert!(ftext(&mut e, "out.algorithm").contains("minimax"));
    // x keeps x0's shape (a row here).
    run(&mut e, "sz = size(x);");
    assert_eq!(values(&e, "sz"), vec![1., 2.]);
}

#[test]
fn fminimax_with_a_linear_constraint() {
    // max(x1^2+x2^2, (x1-2)^2+x2^2) subject to x2 >= 1 (A=[0 -1], b=-1).
    // Both objectives grow with |x2|, so x2 sits on the bound; by symmetry
    // about x1 = 1 the minimax point is (1,1) with value 2.
    let (mut e, _dir) = setup();
    run(
        &mut e,
        "[x,fval,m,flag] = fminimax(@(x)[x(1)^2+x(2)^2; (x(1)-2)^2+x(2)^2],[0;3],[0 -1],-1,[],[],[],[],[],optimset('Display','off'));",
    );
    close(&values(&e, "x"), &[1., 1.], 1e-5, "x");
    assert!((scalar(&e, "m") - 2.).abs() < 1e-5);
    assert!(values(&e, "x")[1] >= 1. - 1e-6);
    assert!(scalar(&e, "flag") > 0.);
}

#[test]
fn fgoalattain_two_objectives_with_known_attainment_factor() {
    // F = ((x-1)^2, (x-3)^2), goal = (0,0), weight = (1,3). gamma(x) =
    // max((x-1)^2, (x-3)^2/3); on [1,3] the first increases, the second
    // decreases, so the optimum equates them: sqrt(3)(x-1) = 3-x, x = sqrt(3),
    // gamma = (sqrt(3)-1)^2 = 4 - 2*sqrt(3).
    let (mut e, _dir) = setup();
    run(
        &mut e,
        "[x,fval,g,flag,out] = fgoalattain(@(x)[(x-1)^2;(x-3)^2],0,[0;0],[1;3],[],[],[],[],[],[],[],optimoptions('fgoalattain','Display','off'));",
    );
    let r3 = 3f64.sqrt();
    assert!((scalar(&e, "x") - r3).abs() < 1e-5);
    assert!((scalar(&e, "g") - (4. - 2. * r3)).abs() < 1e-6);
    close(
        &values(&e, "fval"),
        &[(r3 - 1.).powi(2), (r3 - 3.).powi(2)],
        1e-5,
        "fval",
    );
    assert!(scalar(&e, "flag") > 0.);
    // Over-attainable goals give a negative attainment factor: with goal
    // (2,2) and weight (1,1), gamma(x) = max((x-1)^2, (x-3)^2) - 2 is least
    // where the two parabolas cross, x = 2, so gamma = 1 - 2 = -1.
    run(
        &mut e,
        "[x2,~,g2] = fgoalattain(@(x)[(x-1)^2;(x-3)^2],0,[2;2],[1;1],[],[],[],[],[],[],[],optimset('Display','off'));",
    );
    assert!((scalar(&e, "x2") - 2.).abs() < 1e-5);
    assert!((scalar(&e, "g2") + 1.).abs() < 1e-5);
}

#[test]
fn patternsearch_rosenbrock_reaches_the_minimum() {
    let (mut e, _dir) = setup();
    run(
        &mut e,
        "o = optimoptions('patternsearch','Display','off','MaxIterations',1e5,'MaxFunctionEvaluations',1e6,'MeshTolerance',1e-9);\
         [x,f,flag,out] = patternsearch(@(x)100*(x(2)-x(1)^2)^2+(1-x(1))^2,[-1 2],[],[],[],[],[],[],[],o);",
    );
    close(&values(&e, "x"), &[1., 1.], 1e-3, "x");
    assert_eq!(scalar(&e, "flag"), 1.);
    assert!(fscalar(&mut e, "out.meshsize") <= 1e-9);
    assert_eq!(ftext(&mut e, "out.pollmethod"), "GPSPositiveBasis2N");
}

#[test]
fn patternsearch_bounded_quadratic_hits_the_bound() {
    // (x1+1)^2+(x2-3)^2 on [0,2]^2: the minimum is the corner (0,2). From
    // (1,1) every mesh point is a dyadic step, so the bound is hit exactly.
    let (mut e, _dir) = setup();
    run(
        &mut e,
        "[x,f,flag] = patternsearch(@(x)(x(1)+1)^2+(x(2)-3)^2,[1 1],[],[],[],[],[0 0],[2 2],[],optimset('Display','off'));",
    );
    assert_eq!(values(&e, "x"), vec![0., 2.]);
    assert_eq!(scalar(&e, "f"), 2.);
    assert_eq!(scalar(&e, "flag"), 1.);
}

#[test]
fn patternsearch_linear_and_nonlinear_constraints() {
    let (mut e, _dir) = setup();
    // (x1-2)^2+(x2-2)^2 subject to x1+x2 <= 2: projection of (2,2), i.e. (1,1).
    run(
        &mut e,
        "[x,f,flag,out] = patternsearch(@(x)(x(1)-2)^2+(x(2)-2)^2,[0 0],[1 1],2,[],[],[],[],[],optimset('Display','off'));",
    );
    let x = values(&e, "x");
    assert!(x[0] + x[1] <= 2. + 1e-6, "{x:?}");
    close(&x, &[1., 1.], 1e-3, "inequality");
    assert!(fscalar(&mut e, "out.maxconstraint") <= 1e-6);
    // Equality x1 = x2 with (x1-1)^2+(x2+1)^2: minimum at t = 0 on the line.
    // x0 = (3,1) is infeasible and is projected onto the line first.
    run(
        &mut e,
        "[x,f] = patternsearch(@(x)(x(1)-1)^2+(x(2)+1)^2,[3 1],[],[],[1 -1],0,[],[],[],optimset('Display','off'));",
    );
    let x = values(&e, "x");
    assert!((x[0] - x[1]).abs() <= 1e-6, "{x:?}");
    close(&x, &[0., 0.], 1e-3, "equality");
    // x1+x2 subject to x1^2+x2^2 <= 2: minimum at (-1,-1) (KKT with mu = 1/2).
    run(
        &mut e,
        "[x,f,flag] = patternsearch(@(x)x(1)+x(2),[0 0],[],[],[],[],[],[],@(x)deal(x(1)^2+x(2)^2-2,[]),optimset('Display','off','MaxIter',2000));",
    );
    close(&values(&e, "x"), &[-1., -1.], 1e-3, "nonlcon");
    assert!(scalar(&e, "flag") > 0.);
}

#[test]
fn patternsearch_equality_from_a_degenerate_vertex() {
    // (x1-0.7)^2+(x2-0.3)^2 on x1+x2 = 1, 0 <= x <= 1: the unconstrained
    // minimum (0.7,0.3) is feasible, so it is the optimum. x0 = (0,1) is a
    // vertex where the equality and two bounds are active (3 rows in 2-D):
    // only the tangent direction (1,-1) keeps the equality.
    let (mut e, _dir) = setup();
    run(
        &mut e,
        "f = @(x) (x(1)-0.7)^2 + (x(2)-0.3)^2;\
         [x,fval,flag] = patternsearch(f,[0 1],[],[],[1 1],1,[0 0],[1 1],[],optimset('Display','off'));",
    );
    let x = values(&e, "x");
    close(&x, &[0.7, 0.3], 1e-3, "x");
    assert!((x[0] + x[1] - 1.).abs() <= 1e-9, "{x:?}");
    assert!(scalar(&e, "fval") < 1e-6);
    assert_eq!(scalar(&e, "flag"), 1.);
}

#[test]
fn patternsearch_simplex_from_a_vertex() {
    // sum((x-[0.2 0.3 0.5]).^2) on sum(x) = 1, x >= 0: the unconstrained
    // minimum lies on the simplex, so it is the optimum. x0 = (1,0,0) is a
    // vertex (equality plus two active bounds, and x1 >= 0 within the mesh).
    let (mut e, _dir) = setup();
    run(
        &mut e,
        "[x,fval,flag] = patternsearch(@(x)sum((x-[0.2 0.3 0.5]).^2),[1 0 0],[],[],[1 1 1],1,[0 0 0],[],[],optimset('Display','off'));",
    );
    let x = values(&e, "x");
    close(&x, &[0.2, 0.3, 0.5], 1e-3, "x");
    assert!((x.iter().sum::<f64>() - 1.).abs() <= 1e-9, "{x:?}");
    assert!(x.iter().all(|v| *v >= 0.), "{x:?}");
    assert_eq!(scalar(&e, "flag"), 1.);
}

fn rastrigin_case(e: &mut Engine, call: &str, limit: f64) -> Vec<f64> {
    run(e, &format!("rng(1); [x,f,flag,out] = {call};"));
    let x = values(e, "x");
    let f = scalar(e, "f");
    assert!(f < limit, "{call}: f = {f}, x = {x:?}");
    assert!(x.iter().all(|v| v.abs() <= 5.12), "{x:?}");
    run(e, &format!("rng(1); [x2,f2] = {call};"));
    assert_eq!(values(e, "x2"), x, "{call} is not repeatable");
    assert_eq!(scalar(e, "f2"), f);
    x
}

#[test]
fn particleswarm_rastrigin_with_rng_seed() {
    let (mut e, _dir) = setup();
    let x = rastrigin_case(
        &mut e,
        "particleswarm(@bounded_ras,2,lb,ub,optimoptions('particleswarm','Display','off'))",
        1e-2,
    );
    assert_eq!(x.len(), 2);
    assert_eq!(scalar(&e, "flag"), 1.);
    assert!(fscalar(&mut e, "out.funccount") > 20.);
    // Another seed gives another swarm.
    run(
        &mut e,
        "rng(7); x3 = particleswarm(@bounded_ras,2,lb,ub,optimset('Display','off'));",
    );
    assert_ne!(values(&e, "x3"), x);
}

#[test]
fn ga_rastrigin_with_rng_seed() {
    let (mut e, _dir) = setup();
    rastrigin_case(
        &mut e,
        "ga(@bounded_ras,2,[],[],[],[],lb,ub,[],optimoptions('ga','Display','off'))",
        1e-2,
    );
    assert!(fscalar(&mut e, "out.generations") >= 1.);
    // Population and scores outputs.
    run(
        &mut e,
        "rng(3); [x,f,flag,out,pop,scores] = ga(ras,2,[],[],[],[],lb,ub,[],optimoptions('ga','Display','off','PopulationSize',30,'MaxGenerations',5)); sp = size(pop); ss = size(scores); best = min(scores);",
    );
    assert_eq!(values(&e, "sp"), vec![30., 2.]);
    assert_eq!(values(&e, "ss"), vec![30., 1.]);
    assert_eq!(scalar(&e, "best"), scalar(&e, "f"));
}

#[test]
fn ga_linear_and_nonlinear_constraints() {
    let (mut e, _dir) = setup();
    // (x1-2)^2+(x2-2)^2 subject to x1+x2 <= 2: optimum (1,1), f = 2.
    run(
        &mut e,
        "rng(2); [x,f,flag,out] = ga(@(x)(x(1)-2)^2+(x(2)-2)^2,2,[1 1],2,[],[],[-5 -5],[5 5],[],optimoptions('ga','Display','off'));",
    );
    let x = values(&e, "x");
    assert!(x[0] + x[1] <= 2. + 1e-9, "{x:?}");
    assert!((scalar(&e, "f") - 2.).abs() < 1e-2, "{x:?}");
    // x1+x2 subject to x1^2+x2^2 <= 2 inside [-2,2]^2: optimum (-1,-1).
    run(
        &mut e,
        "rng(2); [x,f,flag,out] = ga(@(x)x(1)+x(2),2,[],[],[],[],[-2 -2],[2 2],@(x)deal(x(1)^2+x(2)^2-2,[]),optimoptions('ga','Display','off'));",
    );
    assert!(fscalar(&mut e, "out.maxconstraint") <= 1e-6);
    assert!(scalar(&e, "f") < -1.9, "f = {}", scalar(&e, "f"));
    assert!(scalar(&e, "flag") >= 0.);
}

#[test]
fn simulannealbnd_rastrigin_with_rng_seed() {
    let (mut e, _dir) = setup();
    rastrigin_case(
        &mut e,
        "simulannealbnd(@bounded_ras,[2 2],lb,ub,optimoptions('simulannealbnd','Display','off'))",
        0.5,
    );
    assert!(fscalar(&mut e, "out.funccount") > 100.);
    run(&mut e, "nt = numel(out.temperature);");
    assert_eq!(scalar(&e, "nt"), 2.);
}

#[test]
fn iteration_limits_stop_with_exitflag_zero() {
    let (mut e, _dir) = setup();
    for (call, field, n) in [
        (
            "particleswarm(ras,2,lb,ub,optimoptions('particleswarm','Display','off','MaxIterations',3))",
            "iterations",
            3.,
        ),
        (
            "ga(ras,2,[],[],[],[],lb,ub,[],optimoptions('ga','Display','off','MaxGenerations',3))",
            "generations",
            3.,
        ),
        (
            "ga(ras,2,[],[],[],[],lb,ub,[],optimset('Display','off','MaxIter',4))",
            "generations",
            4.,
        ),
        (
            "simulannealbnd(ras,[2 2],lb,ub,optimoptions('simulannealbnd','Display','off','MaxIterations',10))",
            "iterations",
            10.,
        ),
        (
            "patternsearch(ras,[2.3 2.3],[],[],[],[],lb,ub,[],optimoptions('patternsearch','Display','off','MaxIterations',2))",
            "iterations",
            2.,
        ),
    ] {
        run(&mut e, &format!("rng(1); [x,f,flag,out] = {call};"));
        assert_eq!(scalar(&e, "flag"), 0., "{call}");
        assert_eq!(fscalar(&mut e, &format!("out.{field}")), n, "{call}");
        assert!(!ftext(&mut e, "out.message").is_empty());
    }
    // Function evaluation budgets also give exitflag 0.
    run(
        &mut e,
        "rng(1); [x,f,flag,out] = simulannealbnd(ras,[2 2],lb,ub,optimoptions('simulannealbnd','Display','off','MaxFunctionEvaluations',25));",
    );
    assert_eq!(scalar(&e, "flag"), 0.);
    assert!(fscalar(&mut e, "out.funccount") <= 25. + 2.);
}

#[test]
fn display_final_and_iter_print() {
    let (mut e, _dir) = setup();
    let out = run(
        &mut e,
        "rng(1); x = particleswarm(ras,2,lb,ub,optimoptions('particleswarm','MaxIterations',2,'Display','iter'));",
    );
    assert!(out.iter().any(|l| l.contains("Iteration 2")), "{out:?}");
    let out = run(
        &mut e,
        "rng(1); x = ga(ras,2,[],[],[],[],lb,ub,[],optimoptions('ga','MaxGenerations',2));",
    );
    assert!(
        out.iter()
            .any(|l| l.contains("Maximum number of generations")),
        "{out:?}"
    );
    let out = run(
        &mut e,
        "x = patternsearch(ras,[2 2],[],[],[],[],[],[],[],optimoptions('patternsearch','Display','off'));",
    );
    assert!(out.is_empty(), "{out:?}");
}

#[test]
fn optimoptions_for_the_new_solvers() {
    let (mut e, _dir) = setup();
    run(
        &mut e,
        "o = optimoptions('ga','PopulationSize',20,'MaxGenerations',7,'MaxStallGenerations',3,'EliteCount',2,'CrossoverFraction',0.5,'FunctionTolerance',1e-8);\
         p = optimoptions('particleswarm','SwarmSize',12,'MaxStallIterations',5);\
         s = optimoptions('simulannealbnd','InitialTemperature',[50 60],'ReannealInterval',30);\
         q = optimoptions('patternsearch','InitialMeshSize',0.5,'MeshTolerance',1e-8,'StepTolerance',1e-9);\
         m = optimoptions('fminimax','Display','off','MaxIterations',50);\
         g = optimoptions('fgoalattain','ConstraintTolerance',1e-8);\
         o2 = optimoptions(o,'PopulationSize',30);",
    );
    assert_eq!(fscalar(&mut e, "o.PopulationSize"), 20.);
    assert_eq!(fscalar(&mut e, "o2.PopulationSize"), 30.);
    assert_eq!(fscalar(&mut e, "o2.MaxGenerations"), 7.);
    assert_eq!(fscalar(&mut e, "p.SwarmSize"), 12.);
    assert_eq!(fvalues(&mut e, "s.InitialTemperature"), vec![50., 60.]);
    assert_eq!(fscalar(&mut e, "q.InitialMeshSize"), 0.5);
    assert_eq!(fscalar(&mut e, "m.MaxIterations"), 50.);
    // Options take effect: PopulationSize sets the population rows.
    run(
        &mut e,
        "rng(1); [x,f,flag,out,pop] = ga(ras,2,[],[],[],[],lb,ub,[],optimoptions(o,'Display','off')); np = size(pop,1);",
    );
    assert_eq!(scalar(&e, "np"), 20.);
    assert!(fscalar(&mut e, "out.generations") <= 7.);
}

#[test]
fn bad_arguments_and_options_are_errors() {
    let (mut e, _dir) = setup();
    for (code, message) in [
        ("ga(@(x)x(1))", "ga: expected fun,nvars"),
        ("ga(@(x)x(1),2,[],[],[],[],[],[],[],[1 2])", "intcon"),
        (
            "particleswarm(@(x)x(1),0)",
            "nvars must be a positive integer",
        ),
        (
            "particleswarm(@(x)x(1),2,[1 1],[0 0])",
            "lb must not exceed ub",
        ),
        (
            "particleswarm(@(x)x(1),2,[1 1 1])",
            "bounds must have 2 entries",
        ),
        (
            "particleswarm(@(x)[x(1);x(2)],2,[-1 -1],[1 1])",
            "real scalar",
        ),
        ("patternsearch(@(x)x,[])", "x0 must not be empty"),
        (
            "patternsearch(@(x)x(1),[1 1],[1 1 1],1)",
            "A,b dimensions do not agree",
        ),
        (
            "patternsearch(@(x)x(1),[0 0],[],[],[],[],[],[],[],7)",
            "options must be a scalar structure",
        ),
        ("simulannealbnd(@(x)x,1,2,1)", "lb must not exceed ub"),
        (
            "simulannealbnd(@(x)x,1,[],[],optimoptions('simulannealbnd','InitialTemperature',[1 2 3]))",
            "InitialTemperature",
        ),
        ("simulannealbnd(@(x)x)", "simulannealbnd: expected fun,x0"),
        ("fminimax(@(x)x)", "fminimax: expected fun,x0"),
        (
            "fgoalattain(@(x)[x;x],0,[0;0],[0;0])",
            "weight must have at least one nonzero",
        ),
        (
            "fgoalattain(@(x)[x;x],0,[0;0;0],[1;1;1])",
            "one entry per objective",
        ),
        ("fgoalattain(@(x)[x;x],0,[0;0])", "fgoalattain: expected"),
        (
            "fminimax(@(x)[x;x],0,[],[],[],[],[],[],[],optimoptions('fminimax','SpecifyObjectiveGradient',true))",
            "SpecifyObjectiveGradient",
        ),
        ("optimoptions('ga','SwarmSize',10)", "does not apply to ga"),
        (
            "optimoptions('particleswarm','SwarmSize',1)",
            "SwarmSize must be an integer between 2",
        ),
        (
            "optimoptions('ga','CrossoverFraction',2)",
            "CrossoverFraction must be between 0 and 1",
        ),
        ("optimoptions('ga','Display','loud')", "Display must be"),
        (
            "ga(@(x)x(1),2,[],[],[],[],[],[],[],optimoptions('particleswarm','SwarmSize',5))",
            "does not apply to ga",
        ),
        (
            "[a,b,c,d,f] = particleswarm(@(x)x(1),2)",
            "does not support 5 outputs",
        ),
    ] {
        let error = fails(&mut e, code);
        assert!(error.contains(message), "{code}: {error}");
    }
}

#[test]
fn cancel_stops_every_solver() {
    let calls = [
        "x = ga(@slow_global,2,[],[],[],[],[],[],[],optimoptions('ga','Display','off','MaxGenerations',Inf));",
        "x = particleswarm(@slow_global,2,[],[],optimoptions('particleswarm','Display','off','MaxIterations',Inf));",
        "x = simulannealbnd(@slow_global,[1 1],[],[],optimoptions('simulannealbnd','Display','off','MaxFunctionEvaluations',Inf));",
        "x = patternsearch(@slow_global,[1 1],[],[],[],[],[],[],[],optimoptions('patternsearch','Display','off','MaxIterations',Inf,'MaxFunctionEvaluations',Inf));",
        "x = fminimax(@(x)[slow_global(x); x(1)],[1 1],[],[],[],[],[],[],[],optimoptions('fminimax','Display','off','MaxIterations',Inf,'MaxFunctionEvaluations',Inf));",
        "x = fgoalattain(@(x)[slow_global(x); x(1)],[1 1],[0 0],[1 1],[],[],[],[],[],[],[],optimoptions('fgoalattain','Display','off','MaxIterations',Inf,'MaxFunctionEvaluations',Inf));",
    ];
    for call in calls {
        let (mut e, _dir) = setup();
        let flag = e.cancellation();
        let setter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let start = Instant::now();
        let error = e.execute(call).expect_err("the run must be cancelled");
        setter.join().unwrap();
        assert!(error.to_lowercase().contains("cancel"), "{call}: {error}");
        assert!(start.elapsed() < Duration::from_secs(20), "{call}");
    }
}

#[test]
fn new_solvers_are_known_functions() {
    let (mut e, _dir) = setup();
    for name in [
        "fminimax",
        "fgoalattain",
        "patternsearch",
        "ga",
        "particleswarm",
        "simulannealbnd",
    ] {
        run(&mut e, &format!("k = exist('{name}');"));
        assert!(scalar(&e, "k") > 0., "{name}");
    }
}
