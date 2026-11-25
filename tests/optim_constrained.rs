//! Independent SciPy fixtures from tool/refs/optim/constrained.py, plus KKT
//! identities and the published HS71 / analytic HS35 optima. No MATLAB evidence.
use matrust::{Array, Engine};
use serde_json::Value as Json;
use std::time::{Duration, Instant};

fn fixture() -> Json {
    serde_json::from_str(include_str!("fixtures/optim/constrained.json")).unwrap()
}
fn vector(j: &Json) -> Vec<f64> {
    j.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect()
}
fn literal(j: &Json) -> String {
    match j {
        Json::Null => "[]".into(),
        Json::Array(rows) => format!(
            "[{}]",
            rows.iter()
                .map(|r| if r.is_array() {
                    vector(r)
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    r.to_string()
                })
                .collect::<Vec<_>>()
                .join(";")
        ),
        _ => j.to_string(),
    }
}
fn values(e: &Engine, n: &str) -> Vec<f64> {
    if let Some(m) = e.variables.get(n) {
        return m.data().to_vec();
    }
    match e.array(n) {
        Some(Array::Numeric(m)) => m.data().to_vec(),
        _ => panic!("missing {n}"),
    }
}
fn text(e: &Engine, n: &str) -> String {
    match e.array(n) {
        Some(Array::Character(c)) => c.to_utf8(&e.cancellation()).unwrap(),
        _ => panic!("missing text {n}"),
    }
}
fn scalar(e: &Engine, n: &str) -> f64 {
    values(e, n)[0]
}
fn run(e: &mut Engine, s: &str) {
    if let Err(err) = e.execute(s) {
        panic!("{s}\n{err}");
    }
}
fn close(a: &[f64], b: &[f64], tol: f64) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b) {
        assert!((a - b).abs() <= tol, "{a} != {b}, tol {tol}");
    }
}
fn objective(e: &Engine, j: &Json) {
    let expected = j["objective"].as_f64().unwrap();
    close(
        &values(e, "f"),
        &[expected],
        (1e-6 * expected.abs()).max(1e-8),
    );
}
fn setup() -> Engine {
    let mut e = Engine::default();
    // The fixture/KKT assertions require tighter accuracy than the default
    // stopping tolerances; request it rather than relying on a hidden cap.
    run(
        &mut e,
        "o=optimoptions('linprog','Display','off','OptimalityTolerance',1e-9,'ConstraintTolerance',1e-9);",
    );
    e
}

#[test]
fn three_linear_programs_and_problem_structure() {
    let fixture = fixture();
    for name in ["lp_textbook", "lp_degenerate", "lp_equality"] {
        let j = &fixture[name];
        let mut e = setup();
        let bounds = j["bounds"].as_array().unwrap();
        let lower: Vec<_> = bounds.iter().map(|p| p[0].to_string()).collect();
        let upper: Vec<_> = bounds
            .iter()
            .map(|p| {
                if p[1].is_null() {
                    "Inf".into()
                } else {
                    p[1].to_string()
                }
            })
            .collect();
        let args = format!(
            "{}, {}, {}, {}, {}, [{}], [{}], o",
            literal(&j["f"]),
            literal(&j["A"]),
            literal(&j["b"]),
            literal(&j["Aeq"]),
            literal(&j["beq"]),
            lower.join(";"),
            upper.join(";")
        );
        run(
            &mut e,
            &format!("[x,f,flag,out,lambda]=linprog({args}); violation=out.constrviolation;"),
        );
        assert_eq!(scalar(&e, "flag"), 1., "{name}");
        objective(&e, j);
        run(
            &mut e,
            &format!(
                "station={}+{}'*lambda.ineqlin+{}'*lambda.eqlin-lambda.lower+lambda.upper; cv=out.constrviolation;",
                literal(&j["f"]),
                literal(&j["A"]),
                if j["Aeq"].is_null() {
                    format!("zeros(0,{})", j["f"].as_array().unwrap().len())
                } else {
                    literal(&j["Aeq"])
                }
            ),
        );
        close(
            &values(&e, "station"),
            &vec![0.; j["f"].as_array().unwrap().len()],
            1e-6,
        );
        assert!(scalar(&e, "cv") < 1e-6);

        assert!(scalar(&e, "violation") < 1e-7);
        run(
            &mut e,
            &format!(
                "comp=lambda.ineqlin.*({}*x-{}); zz=[lambda.ineqlin;lambda.lower;lambda.upper];",
                literal(&j["A"]),
                literal(&j["b"])
            ),
        );
        for v in values(&e, "comp") {
            assert!(v.abs() < 1e-6);
        }
        assert!(values(&e, "zz").iter().all(|v| *v >= -1e-9));
        if j["unique_lambda"].as_bool().unwrap_or(false)
            && j["strict_complementarity"].as_bool().unwrap_or(false)
        {
            run(
                &mut e,
                "zi=lambda.ineqlin; ye=lambda.eqlin; zl=lambda.lower; zu=lambda.upper;",
            );
            for (v, k) in [
                ("zi", "ineqlin"),
                ("ye", "eqlin"),
                ("zl", "lower"),
                ("zu", "upper"),
            ] {
                close(&values(&e, v), &vector(&j[k]), 1e-5);
            }
        }

        // Compare coordinates only when the script proves a unique primal optimum.
        if j["unique_x"].as_bool().unwrap_or(false) {
            close(&values(&e, "x"), &vector(&j["x"]), 1e-5);
        }
    }
    let mut e = setup();
    run(
        &mut e,
        "p=struct('f',[-3;-2],'Aineq',[1 1;1 0;0 1],'bineq',[4;2;3],'lb',[0;0],'options',o); [x,f,flag]=linprog(p);",
    );
    close(&values(&e, "x"), &[2., 2.], 1e-8);
    assert_eq!(scalar(&e, "flag"), 1.);
}
#[test]
fn infeasible_and_unbounded_have_distinct_flags() {
    let mut e = setup();
    for call in [
        "linprog(1,[1;-1],[0;-1],[],[],[],[],o)",
        "linprog(1,[],[],[],[],2,1,o)",
        "linprog([1;1],[],[],[1 1;2 2],[1;3],[],[],o)",
    ] {
        run(&mut e, &format!("[x,f,flag]={call};"));
        assert_eq!(scalar(&e, "flag"), -2., "{call}");
    }
    for call in [
        "linprog(-1,-1,0,[],[],[],[],o)",
        "linprog(-1,[],[],[],[],[],[],o)",
        "linprog([-1;0],[],[],[0 1],0,[],[],o)",
    ] {
        run(&mut e, &format!("[x,f,flag]={call};"));
        assert_eq!(scalar(&e, "flag"), -3., "{call}");
        assert!(values(&e, "x").is_empty());
        assert!(values(&e, "f").is_empty());
    }
}
#[test]
fn convex_quadratics_match_scipy_and_analytic_kkt() {
    let fixture = fixture();
    let mut e = setup();
    for name in ["qp_inequality", "qp_equality"] {
        let j = &fixture[name];
        run(
            &mut e,
            &format!(
                "[x,f,flag,out,lambda]=quadprog({}, {}, {}, {}, {}, {}, [], [], [], o);",
                literal(&j["H"]),
                literal(&j["f"]),
                literal(&j["A"]),
                literal(&j["b"]),
                literal(&j["Aeq"]),
                literal(&j["beq"])
            ),
        );
        assert_eq!(scalar(&e, "flag"), 1.);
        objective(&e, j);
        if j["unique_x"].as_bool().unwrap_or(false) {
            close(&values(&e, "x"), &vector(&j["x"]), 1e-5);
        }
        run(&mut e, "cv=out.constrviolation; fo=out.firstorderopt;");
        assert!(scalar(&e, "cv") < 1e-6 && scalar(&e, "fo") < 1e-6);
        // Analytic KKT: (0.5,1.5), z=1; equality solution (0.25,0.75), y=-0.75.
        if name == "qp_inequality" {
            close(&values(&e, "x"), &[0.5, 1.5], 1e-8);
            run(&mut e, "z=lambda.ineqlin;");
            close(&values(&e, "z"), &[1.], 1e-8);
        } else {
            close(&values(&e, "x"), &[0.25, 0.75], 1e-8);
            run(&mut e, "y=lambda.eqlin;");
            close(&values(&e, "y"), &[-0.75], 1e-8);
        }
    }
    let err = e
        .execute("quadprog(diag([1;1;-1.2e-10]),zeros(3,1))")
        .unwrap_err();
    assert!(err.contains("positive semidefinite"));
    let err = e.execute("quadprog([1 0;0 -1],[0;0],[],[])").unwrap_err();
    assert!(err.contains("quadprog: H must be positive semidefinite for this solver"));
    run(
        &mut e,
        "[x,f,flag]=quadprog([2 0;0 0],[-2;0],[],[],[],[],[0;0],[2;2],[],o);",
    );
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&[scalar(&e, "x")], &[1.], 1e-7);
}
#[test]
fn multipliers_obey_lagrangian_signs() {
    let mut e = setup();
    run(
        &mut e,
        "[x,f,flag,out,L]=linprog([-1;2;3],[],[],[0 0 1],4,[0;1;-Inf],[2;Inf;Inf],o); lo=L.lower; up=L.upper; eq=L.eqlin; station=[-1;2;3]-lo+up+[0;0;1]*eq;",
    );
    close(&values(&e, "lo"), &[0., 2., 0.], 1e-7);
    close(&values(&e, "up"), &[1., 0., 0.], 1e-7);
    close(&values(&e, "eq"), &[-3.], 1e-7);
    close(&values(&e, "station"), &[0.; 3], 1e-7);
    run(
        &mut e,
        "[x,f,flag,out,L]=quadprog(2*eye(2),[-4;2],[],[],[],[],[0;0],[1;2],[],o); lo=L.lower; up=L.upper; station=2*x+[-4;2]-lo+up;",
    );
    close(&values(&e, "lo"), &[0., 2.], 1e-7);
    close(&values(&e, "up"), &[2., 0.], 1e-7);
    close(&values(&e, "station"), &[0.; 2], 1e-7);
}
#[test]
fn bounded_least_squares_matches_lsq_linear() {
    let j = &fixture()["lsqlin"];
    let mut e = setup();
    run(
        &mut e,
        &format!(
            "[x,rn,res,flag,out,L]=lsqlin({}, {}, [], [], [], [], [0;0], [1;2], [], o);",
            literal(&j["C"]),
            literal(&j["d"])
        ),
    );
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &vector(&j["x"]), 1e-5);
    close(&values(&e, "rn"), &[j["resnorm"].as_f64().unwrap()], 1e-6);
    close(&values(&e, "res"), &vector(&j["residual"]), 1e-5);
    run(
        &mut e,
        &format!(
            "station={}'*res-L.lower+L.upper; cv=out.constrviolation;",
            literal(&j["C"])
        ),
    );
    close(&values(&e, "station"), &[0.; 2], 1e-6);
    assert!(scalar(&e, "cv") < 1e-6);
}
fn callbacks(e: &mut Engine, dir: &std::path::Path) {
    std::fs::write(
        dir.join("disk.m"),
        "function [c,ceq]=disk(x)\nc=x'*x-2; ceq=[];\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("hs71con.m"),
        "function [c,ceq]=hs71con(x)\nc=25-prod(x); ceq=sum(x.^2)-40;\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("eqcon.m"),
        "function [c,ceq,GC,GE]=eqcon(x)\nc=[]; ceq=x(1)+x(2)-1; GC=[]; GE=[1;1];\nend\n",
    )
    .unwrap();
    std::fs::write(dir.join("objective_gradient.m"),"function [f,g]=objective_gradient(x)\nf=(x(1)-2)^2+(x(2)-3)^2; g=[2*(x(1)-2);2*(x(2)-3)];\nend\n").unwrap();
    e.set_function_directory(dir).unwrap();
}
#[test]
fn sqp_rosenbrock_disk_and_hock_schittkowski() {
    let fixture = fixture();
    let mut e = setup();
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    callbacks(&mut e, dir.path());
    let cases = [
        (
            "rosen_disk",
            "fmincon(@(x) 100*(x(2)-x(1)^2)^2+(1-x(1))^2,[-1.2;1],[],[],[],[],[],[],@disk,o)",
        ),
        (
            "rosen_disk_binding",
            "fmincon(@(x) 100*(x(2)-x(1)^2)^2+(1-x(1))^2,[-1.2;1],[],[],[],[],[],[],@(x) deal(x'*x-1,[]),o)",
        ),
        (
            "hs71",
            "fmincon(@(x) x(1)*x(4)*(x(1)+x(2)+x(3))+x(3),[1;5;5;1],[],[],[],[],ones(4,1),5*ones(4,1),@hs71con,o)",
        ),
        (
            "hs35",
            "fmincon(@(x) 9-8*x(1)-6*x(2)-4*x(3)+2*x(1)^2+2*x(2)^2+x(3)^2+2*x(1)*x(2)+2*x(1)*x(3),[.5;.5;.5],[1 1 2],3,[],[],zeros(3,1),[],[],o)",
        ),
        (
            "nonlinear_equality",
            "fmincon(@(x) (x(1)-2)^2+(x(2)-3)^2,[0;0],[],[],[],[],[],[],@eqcon,o)",
        ),
    ];
    for difference in ["forward", "central"] {
        run(
            &mut e,
            &format!(
                "o=optimoptions('fmincon','Display','off','FiniteDifferenceType','{difference}','MaxIterations',500,'MaxFunctionEvaluations',20000,'OptimalityTolerance',1e-8,'ConstraintTolerance',1e-8);"
            ),
        );
        for (name, call) in cases {
            run(
                &mut e,
                &format!(
                    "[x,f,flag,out,L,g,H]={call}; cv=out.constrviolation; fo=out.firstorderopt; it=out.iterations;"
                ),
            );
            assert!(
                scalar(&e, "flag") > 0.,
                "{name}/{difference}: flag={} f={} x={:?} violation={} optimality={} iterations={}",
                scalar(&e, "flag"),
                scalar(&e, "f"),
                values(&e, "x"),
                scalar(&e, "cv"),
                scalar(&e, "fo"),
                scalar(&e, "it")
            );
            objective(&e, &fixture[name]);
            let checks = match name {
                "rosen_disk" => "station=g+2*x*L.ineqnonlin; comp=L.ineqnonlin*(x'*x-2);",
                "rosen_disk_binding" => "station=g+2*x*L.ineqnonlin; comp=L.ineqnonlin*(x'*x-1);",
                "hs71" => {
                    "station=g-prod(x)./x*L.ineqnonlin+2*x*L.eqnonlin-L.lower+L.upper; comp=[L.ineqnonlin*(25-prod(x));L.lower.*(1-x);L.upper.*(x-5)];"
                }
                "hs35" => {
                    "station=g+[1;1;2]*L.ineqlin-L.lower+L.upper; comp=[L.ineqlin*([1 1 2]*x-3);L.lower.*(-x)];"
                }
                _ => "station=g+ones(2,1)*L.eqnonlin; comp=0;",
            };
            run(&mut e, checks);
            close(
                &values(&e, "station"),
                &vec![0.; values(&e, "x").len()],
                5e-6,
            );
            for v in values(&e, "comp") {
                assert!(v.abs() < 1e-6, "{name}: complementarity {v}");
            }
            run(&mut e, "zall=[L.ineqlin;L.lower;L.upper;L.ineqnonlin];");
            assert!(values(&e, "zall").iter().all(|v| *v >= -1e-9));

            if fixture[name]["unique_x"].as_bool().unwrap_or(false) {
                close(&values(&e, "x"), &vector(&fixture[name]["x"]), 1e-5);
            }
            assert!(
                scalar(&e, "fo") < 5e-6,
                "{name}: stationarity {}",
                scalar(&e, "fo")
            );
            assert!(scalar(&e, "cv") < 1e-7);
        }
    }
}
#[test]
fn sqp_user_gradients_and_alias_notes() {
    let mut e = setup();
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    callbacks(&mut e, dir.path());
    run(
        &mut e,
        "o=optimoptions('fmincon','Display','off','SpecifyObjectiveGradient',true,'SpecifyConstraintGradient',true,'Algorithm','interior-point'); [x,f,flag,out,L]=fmincon(@objective_gradient,[0;0],[],[],[],[],[],[],@eqcon,o); msg=out.message; eq=L.eqnonlin;",
    );
    close(&values(&e, "x"), &[0., 1.], 1e-6);
    assert_eq!(scalar(&e, "flag"), 1.);
    assert!(text(&e, "msg").contains("mapped"));
    close(&values(&e, "eq"), &[4.], 1e-6);
    run(
        &mut e,
        "o=optimoptions('linprog','Display','off','Algorithm','dual-simplex'); [x,f,flag,out]=linprog(-1,1,2,[],[],0,[],o); msg=out.message;",
    );
    assert!(text(&e, "msg").contains("mapped"));
}
#[test]
fn integer_knapsack_assignment_and_node_limit() {
    let fixture = fixture();
    let mut e = setup();
    for name in ["knapsack", "assignment"] {
        let j = &fixture[name];
        let n = j["f"].as_array().unwrap().len();
        run(
            &mut e,
            &format!(
                "[x,f,flag,out]=intlinprog({},1:{n},{},{},{},{},zeros({n},1),ones({n},1),o);",
                literal(&j["f"]),
                literal(&j["A"]),
                literal(&j["b"]),
                literal(&j["Aeq"]),
                literal(&j["beq"])
            ),
        );
        assert_eq!(scalar(&e, "flag"), 1., "{name}");
        objective(&e, j);
        run(&mut e, "cv=out.constrviolation; gap=out.absolutegap;");
        assert!(scalar(&e, "cv") < 1e-6 && scalar(&e, "gap") < 1e-6);
        assert!(j["second_best_objective"].as_f64().unwrap() > j["objective"].as_f64().unwrap());

        if j["unique_x"].as_bool().unwrap_or(false) {
            close(&values(&e, "x"), &vector(&j["x"]), 1e-5);
        }
    }
    run(
        &mut e,
        "o=optimoptions('intlinprog','MaxNodes',0,'Display','off'); [x,f,flag,out]=intlinprog(-1,1,1,2,[],[],0,3,o); nodes=out.iterations;",
    );
    assert_eq!(scalar(&e, "flag"), 0.);
    assert_eq!(scalar(&e, "nodes"), 0.);
    // Root x=1.5, first child x=1 is feasible, sibling x>=2 remains queued.
    run(
        &mut e,
        "o.MaxNodes=2; [x,f,flag]=intlinprog(-1,1,1,1.5,[],[],0,3,o);",
    );
    assert_eq!(scalar(&e, "flag"), 2.);
    close(&values(&e, "x"), &[1.], 1e-7);
}
#[test]
fn checked_inputs_limits_and_registration() {
    let mut e = setup();
    for (source, part) in [
        ("linprog([1;2],1,2)", "dimensions"),
        ("quadprog(eye(2),[1;2;3],[],[])", "dimensions"),
        ("intlinprog(1,2,[],[])", "intcon"),
        ("linprog(NaN,[],[])", "finite"),
        ("linprog(ones(2001,1),[],[])", "limit"),
        (
            "linprog(1,[],[],[],[],[],[],optimset('PlotFcns',1))",
            "PlotFcns",
        ),
        (
            "linprog(1,[],[],[],[],[],[],optimoptions('linprog','Algorithm','bogus'))",
            "Algorithm",
        ),
        (
            "optimoptions('linprog','ConstraintTolerance',0)",
            "ConstraintTolerance",
        ),
    ] {
        let err = e.execute(source).expect_err(source);
        assert!(err.contains(part), "{source}: {err}");
    }
    for name in ["linprog", "quadprog", "lsqlin", "fmincon", "intlinprog"] {
        assert!(matrust::session::FUNCTIONS.contains(&name));
        run(&mut e, &format!("present=exist('{name}');"));
        assert_eq!(scalar(&e, "present"), 5.);
    }
}
#[test]
fn cancel_stops_a_long_fmincon_callback() {
    let mut e = setup();
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    std::fs::write(dir.path().join("slow_con_objective.m"),"function f=slow_con_objective(x)\nk=0; while k<1000000\nf=(x-3)^2+rand(); k=k+1;\nend\nend\n").unwrap();
    e.set_function_directory(dir.path()).unwrap();
    let flag = e.cancellation();
    let setter = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let start = Instant::now();
    let err=e.execute("x=fmincon(@slow_con_objective,0,[],[],[],[],[],[],[],optimoptions('fmincon','Display','off','MaxIterations',Inf,'MaxFunctionEvaluations',Inf));").expect_err("must cancel");
    setter.join().unwrap();
    assert!(err.to_lowercase().contains("cancel"), "{err}");
    assert!(start.elapsed() < Duration::from_secs(20));
}

#[test]
fn amended_forms_presolve_and_algorithm_aliases() {
    let mut e = setup();
    for call in [
        "linprog(struct('f',1,'Aineq',[1;-1],'bineq',[2;3]))",
        "quadprog(struct('H',2,'f',-2,'lb',0,'ub',2))",
        "lsqlin(struct('C',1,'d',2,'lb',0,'ub',1))",
        "fmincon(struct('objective',@(x) (x-1)^2,'x0',0,'Aineq',[],'bineq',[]))",
        "intlinprog(struct('f',-1,'intcon',1,'lb',0.1,'ub',2.9))",
        "quadprog(2,-2)",
        "lsqlin(1,2,[],[])",
        "fmincon(@(x) (x-1)^2,0,[],[])",
        "linprog([],1,2,[],[],0,1)",
        "quadprog(2,[])",
    ] {
        run(&mut e, &format!("[x,f,flag]={call};"));
        if call.starts_with("lsqlin") {
            run(&mut e, &format!("[x,f,res,flag]={call};"));
        }
        assert!(scalar(&e, "flag") > 0., "{call}: {}", scalar(&e, "flag"));
    }
    run(
        &mut e,
        "[x,f,flag,out,L]=linprog([1;2],[],[],[1 1;2 2],[3;6],[1;-Inf],[1;Inf]); station=[1;2]+[1 1;2 2]'*L.eqlin-L.lower+L.upper;",
    );
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[1., 2.], 1e-6);
    close(&values(&e, "station"), &[0., 0.], 1e-6);
    run(&mut e, "[x,f,flag]=quadprog([2 2;-2 2],[-2;-4]);");
    close(&values(&e, "x"), &[1., 2.], 1e-5);
    for name in [
        "interior-point",
        "interior-point-legacy",
        "dual-simplex",
        "sqp",
        "sqp-legacy",
        "active-set",
    ] {
        run(
            &mut e,
            &format!(
                "op=optimoptions('quadprog','Algorithm','{name}','Display','off'); [x,f,flag,out]=quadprog(2,-2,[],[],[],[],0,2,[],op); msg=out.message;"
            ),
        );
        assert_eq!(scalar(&e, "flag"), 1.);
        if name != "interior-point" {
            assert!(text(&e, "msg").contains("mapped"));
        }
    }
    run(
        &mut e,
        "op=optimoptions('intlinprog','IntegerTolerance',1e-6,'RelativeGapTolerance',0,'AbsoluteGapTolerance',0,'MaxNodes',20); [x,f,flag,out]=intlinprog(-1,1,[],[],[],[],0.1,2.9,op); gap=out.absolutegap;",
    );
    close(&values(&e, "x"), &[2.], 1e-12);
    assert_eq!(scalar(&e, "flag"), 1.);
    assert!(scalar(&e, "gap") <= 1e-6);
    run(&mut e, "[x,f,flag]=intlinprog(-1,1,[],[],[],[],0,Inf);");
    assert_eq!(scalar(&e, "flag"), -3.);
}

#[test]
fn sqp_bounds_and_objective_only_counter() {
    let mut e = setup();
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    std::fs::write(dir.path().join("bounded_objective.m"),"function [f,g]=bounded_objective(x)\nassert(x>=0); assert(x<=1e-12); f=(x-5e-13)^2; g=2*(x-5e-13);\nend\n").unwrap();
    std::fs::write(dir.path().join("bounded_constraint.m"),"function [c,ceq]=bounded_constraint(x)\nassert(x>=0); assert(x<=1e-12); c=x-1e-12; ceq=[];\nend\n").unwrap();
    e.set_function_directory(dir.path()).unwrap();
    for diff in ["forward", "central"] {
        run(
            &mut e,
            &format!(
                "op=optimoptions('fmincon','Display','off','FiniteDifferenceType','{diff}'); [x,f,flag,out]=fmincon(@bounded_objective,3,[],[],[],[],0,1e-12,@bounded_constraint,op); fc=out.funcCount;"
            ),
        );
        assert!(scalar(&e, "flag") > 0.);
        assert!(scalar(&e, "x") >= 0. && scalar(&e, "x") <= 1e-12);
        assert_eq!(scalar(&e, "fc"), 2.); // fun(x) and one inward difference, no constraint calls
    }
    run(
        &mut e,
        "op=optimoptions('fmincon','MaxIterations',0,'SpecifyObjectiveGradient',true,'Display','off'); [x,f,flag,out]=fmincon(@bounded_objective,-3,[],[],[],[],0,1e-12,@bounded_constraint,op); fc=out.funcCount;",
    );
    assert_eq!(scalar(&e, "flag"), 0.);
    assert_eq!(scalar(&e, "fc"), 1.);
    close(&values(&e, "x"), &[0.], 0.);
}

#[test]
fn unresolved_integer_relaxations_do_not_prove_infeasibility() {
    let mut e = setup();
    run(
        &mut e,
        "op=optimoptions('intlinprog','MaxIterations',0,'MaxNodes',10,'Display','off'); [x,f,flag,out]=intlinprog(-1,1,1,1.5,[],[],0,3,op); nodes=out.numnodes;",
    );
    assert_eq!(scalar(&e, "flag"), 0.);
    assert_eq!(scalar(&e, "nodes"), 1.);
    run(&mut e, "[x,f,flag]=intlinprog(1,1,[],[],[],[],0.2,0.8);");
    assert_eq!(scalar(&e, "flag"), -2.);
}

#[test]
#[ignore = "manual release timings, run separately with RAYON_NUM_THREADS=1 and 4"]
fn ocn_release_timings() {
    use std::hint::black_box;
    let workers = std::env::var("RAYON_NUM_THREADS")
        .unwrap_or_else(|_| "1".into())
        .parse::<usize>()
        .expect("RAYON_NUM_THREADS must be an integer");
    matrust::initialize_worker_pool(workers).expect("configure benchmark workers");
    println!("workers={}", rayon::current_num_threads());
    for (label, script, reps) in [
        (
            "LP n=2000, finite box",
            "[x,v,flag]=linprog(ones(2000,1),[],[],[],[],zeros(2000,1),ones(2000,1));",
            3,
        ),
        (
            "LS n=700, square identity",
            "[x,v,res,flag]=lsqlin(eye(700),ones(700,1));",
            3,
        ),
        (
            "LS n=2000, one data row",
            "[x,v,res,flag]=lsqlin(ones(1,2000),1);",
            3,
        ),
        (
            "LP n=2, wide bounds/tiny cost",
            "[x,v,flag]=linprog([1e-8;1],[],[],[],[],[-1e6;0],[1e6;3]);",
            20,
        ),
        (
            "MILP n=1, infeasible near-integer rounding",
            "[x,v,flag]=intlinprog(-1,1,1,0.999999,[],[],0,2);",
            20,
        ),
        (
            "LP n=3, equality+bounds",
            "[x,v,flag]=linprog([1;2;3],[-1 -1 0],-1,[1 1 1],3,[0;0;0],[2;2;2],o);",
            20,
        ),
        (
            "LS n=2, residuals=3, bounds",
            "[x,v,res,flag]=lsqlin([1 2;2 -1;1 1],[4;1;3],[],[],[],[],[0;0],[1;2],[],o);",
            20,
        ),
        (
            "LS n=2, data rows=1200, QR compression",
            "[x,v,res,flag]=lsqlin(ones(1200,2),ones(1200,1));",
            20,
        ),
        (
            "LS n=2, data rows=4000, QR compression",
            "[x,v,res,flag]=lsqlin(ones(4000,2),ones(4000,1));",
            20,
        ),
        (
            "SQP n=1, objective domain backtracking",
            "[x,v,flag]=fmincon(@(x) (log(x)+x)^2,1,[],[],[],[],-2,2);",
            20,
        ),
        (
            "SQP n=2, binding disk",
            "[x,v,flag]=fmincon(@(x) 100*(x(2)-x(1)^2)^2+(1-x(1))^2,[-1.2;1],[],[],[],[],[],[],@(x) deal(x'*x-1,[]),o);",
            5,
        ),
        (
            "MILP n=4 binary, knapsack",
            "[x,v,flag]=intlinprog([-10;-7;-12;-8],1:4,[4 3 6 4],9,[],[],zeros(4,1),ones(4,1),o);",
            5,
        ),
    ] {
        let mut e = setup();
        run(&mut e, script);
        let start = Instant::now();
        for _ in 0..reps {
            run(&mut e, script);
            black_box(scalar(&e, "v"));
            assert!(scalar(&e, "flag") > 0.);
        }
        println!(
            "{label}, repetitions={reps}, total_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.
        );
    }
    for n in [16, 64, 128] {
        let mut e = setup();
        run(
            &mut e,
            &format!("H=2*eye({n}); f=-ones({n},1); lb=zeros({n},1); ub=ones({n},1);"),
        );
        let script = "[x,v,flag]=quadprog(H,f,[],[],[],[],lb,ub,[],o);";
        run(&mut e, script);
        let start = Instant::now();
        for _ in 0..5 {
            run(&mut e, script);
            black_box(scalar(&e, "v"));
            assert_eq!(scalar(&e, "flag"), 1.);
        }
        println!(
            "QP n={n}, 2n bound rows, repetitions=5, total_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.
        );
    }
}

#[test]
fn homogeneous_both_infeasible_certificate_and_elastic_sqp() {
    let mut e = setup();
    run(&mut e, "[x,f,flag]=linprog(-1e10,1,1,[],[],0,[]);");
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[1.], 1e-6);
    // x2 <= 0, x2 >= 1 has a Farkas certificate; x1 is a recession ray.
    run(&mut e, "[x,f,flag]=linprog([-1;0],[0 1;0 -1],[0;-1]);");
    assert_eq!(scalar(&e, "flag"), -5.);
    run(
        &mut e,
        "[x,f,flag]=intlinprog([-1;0],1,[],[],[],[],[0;2],[Inf;2]);",
    );
    assert_eq!(scalar(&e, "flag"), -3.);
    // Inconsistent initial linearisation 0*d=1; the elastic objective supplies
    // an improving step, then the nonlinear equality becomes regular.
    run(
        &mut e,
        "[x,f,flag,out]=fmincon(@(x) (x-2)^2,0,[],[],[],[],-2,2,@(x) deal([],x^2-1),o); cv=out.constrviolation;",
    );
    assert!(scalar(&e, "flag") > 0.);
    close(&values(&e, "x"), &[1.], 1e-5);
    close(&values(&e, "f"), &[1.], 1e-6);
    assert!(scalar(&e, "cv") < 1e-6);
}

fn empty_solution(e: &mut Engine, names: &[&str], lambda: bool) {
    for name in names {
        assert!(values(e, name).is_empty(), "{name} must be []");
    }
    if lambda {
        for field in [
            "ineqlin",
            "eqlin",
            "lower",
            "upper",
            "ineqnonlin",
            "eqnonlin",
        ] {
            run(e, &format!("part=L.{field};"));
            assert!(values(e, "part").is_empty(), "lambda.{field} must be []");
        }
    }
}

#[test]
fn failed_solves_do_not_expose_placeholders_or_certificates_as_points() {
    let mut e = setup();
    // Linear contradictions and zero-curvature recession directions follow
    // directly from Ax<=b and Qd=0, c'd<0 (Nocedal & Wright, ch. 16).
    for (call, flag) in [
        ("linprog(1,[1;-1],[0;-1])", -2.),
        ("linprog(1,[],[],[],[],2,1)", -2.),
        ("linprog([-1;0],[],[],[],[],[0;2],[Inf;2])", -3.),
        ("linprog([-1;0],[0 1;0 -1],[0;-1])", -5.),
        ("quadprog(2,0,[1;-1],[0;-1])", -2.),
        ("quadprog([2 0;0 0],[0;-1])", -3.),
        (
            "quadprog(2,0,[],[],1,1,[],[],0,optimoptions('quadprog','MaxIterations',0))",
            0.,
        ),
        (
            "linprog(-1,1,0,[],[],[],[],optimoptions('linprog','MaxIterations',0))",
            0.,
        ),
    ] {
        run(&mut e, &format!("[x,f,flag,out,L]={call};"));
        assert_eq!(scalar(&e, "flag"), flag, "{call}");
        empty_solution(&mut e, &["x", "f"], true);
        run(&mut e, &format!("single={call};"));
        empty_solution(&mut e, &["single"], false);
    }
    for (call, flag) in [
        ("lsqlin(1,0,[1;-1],[0;-1])", -2.),
        (
            "lsqlin(1,0,[],[],[],[],[],[],1e200,optimoptions('lsqlin','MaxIterations',0))",
            0.,
        ),
        (
            "lsqlin(1,0,[],[],1,1,[],[],0,optimoptions('lsqlin','MaxIterations',0))",
            0.,
        ),
    ] {
        run(&mut e, &format!("[x,rn,res,flag,out,L]={call};"));
        assert_eq!(scalar(&e, "flag"), flag);
        empty_solution(&mut e, &["x", "rn", "res"], true);
    }
    for (call, flag) in [
        ("intlinprog(1,1,[],[],[],[],0.2,0.8)", -2.),
        ("intlinprog(-1,1,[],[],[],[],0,Inf)", -3.),
        (
            "intlinprog(-1,1,[],[],[],[],0,2,optimoptions('intlinprog','MaxNodes',0))",
            0.,
        ),
    ] {
        run(&mut e, &format!("[x,f,flag,out]={call};"));
        assert_eq!(scalar(&e, "flag"), flag);
        empty_solution(&mut e, &["x", "f"], false);
    }
    run(
        &mut e,
        "[x,f,flag,out,L,g,H]=fmincon(@(x) deal(x^2,2*x),0,[],[],[],[],[],[],@(x) deal([],1),optimoptions('fmincon','SpecifyObjectiveGradient',true));",
    );
    assert_eq!(scalar(&e, "flag"), -2.);
    empty_solution(&mut e, &["x", "f", "g", "H"], true);
    run(
        &mut e,
        "[x,f,flag,out,L,g,H]=fmincon(@(x) x^2,0,[],[],[],[],[],[],[],optimoptions('fmincon','MaxFunctionEvaluations',0));",
    );
    assert_eq!(scalar(&e, "flag"), 0.);
    empty_solution(&mut e, &["x", "f", "g", "H"], true);

    // A real feasible iterate/incumbent remains useful at a budget stop.
    run(
        &mut e,
        "[x,f,flag]=quadprog(2,-2,[],[],[],[],0,2,0.5,optimoptions('quadprog','MaxIterations',0));",
    );
    assert_eq!(scalar(&e, "flag"), 0.);
    close(&values(&e, "x"), &[0.5], 0.);
    close(&values(&e, "f"), &[-0.75], 0.);
    run(
        &mut e,
        "[x,f,flag]=intlinprog(-1,1,1,1.5,[],[],0,3,optimoptions('intlinprog','MaxNodes',2));",
    );
    assert_eq!(scalar(&e, "flag"), 2.);
    close(&values(&e, "x"), &[1.], 0.);
    close(&values(&e, "f"), &[-1.], 0.);
}

#[test]
fn user_optimality_tolerance_controls_qp_iterations_independently_of_feasibility() {
    let mut e = setup();
    let mut iterations = Vec::new();
    for tol in [1e-4, 1e-10] {
        run(
            &mut e,
            &format!(
                "op=optimoptions('quadprog','Display','off','OptimalityTolerance',{tol},'ConstraintTolerance',1e-10); [x,f,flag,out,L]=quadprog(2,-2,[],[],[],[],-3,3,0,op); it=out.iterations; cv=out.constrviolation; fo=out.firstorderopt; station=2*x-2-L.lower+L.upper; comp=[L.lower*(x+3);L.upper*(3-x)];"
            ),
        );
        assert_eq!(scalar(&e, "flag"), 1.);
        assert!(scalar(&e, "cv") <= 1e-10);
        assert!(scalar(&e, "fo") <= 3. * tol);
        assert!(scalar(&e, "station").abs() <= 3. * tol);
        assert!(values(&e, "comp").iter().all(|v| v.abs() <= 6. * tol));
        // Strictly convex quadratic: derivative 2*x-2=0 uniquely gives x=1, f=-1.
        close(&values(&e, "x"), &[1.], 10. * tol);
        close(&values(&e, "f"), &[-1.], 10. * tol);
        iterations.push(scalar(&e, "it"));
    }
    assert!(
        iterations[0] < iterations[1],
        "loose/tight iterations: {iterations:?}"
    );
}

#[test]
fn returned_qp_options_clear_legacy_or_canonical_optimality_tolerance_in_a_solve() {
    // min x^2-2x on [-3,3] from 0 with MaxIter=3: the default optimality
    // tolerance meets the cap ([flag iterations]=[0 3]); a .01 one stops at
    // [1 2]. Either alias set to .01 and the other cleared to [] must solve as
    // the default, in two update calls or one; sources stay unchanged.
    let mut e = Engine::default();
    run(&mut e, "o=optimoptions('quadprog','MaxIter',3);");
    let (default, loose) = ([0., 3.], [1., 2.]);
    for (update, expected) in [
        ("u=o;", default),
        ("p=optimoptions(o,'TolFun',.01); u=p;", loose),
        ("u=optimoptions(p,'OptimalityTolerance',[]);", default),
        (
            "u=optimoptions(o,'TolFun',.01,'OptimalityTolerance',[]);",
            default,
        ),
        ("c=optimoptions(o,'OptimalityTolerance',.01); u=c;", loose),
        ("u=optimoptions(c,'TolFun',[]);", default),
        (
            "u=optimoptions(o,'OptimalityTolerance',.01,'TolFun',[]);",
            default,
        ),
        ("u=o;", default),
        ("u=p;", loose),
        ("u=c;", loose),
    ] {
        run(
            &mut e,
            &format!(
                "{update} [x,f,k,r]=quadprog(2,-2,[],[],[],[],-3,3,0,u); kr=[k r.iterations]; \
                 tolset=double(isfield(u,'OptimalityTolerance') && ~isempty(u.OptimalityTolerance));"
            ),
        );
        assert_eq!(values(&e, "kr"), expected, "{update}");
        assert_eq!(scalar(&e, "tolset") == 1., expected == loose, "{update}");
        // Strictly convex: 2*x-2=0 gives x=1; both stops lie within .01 of it.
        close(&values(&e, "x"), &[1.], 1e-2);
    }
}

#[test]
fn lp_gap_accounts_for_wide_bounds_and_small_costs() {
    let mut e = setup();
    // Weak duality/KKT (Wright 1997, ch. 9): positive costs put both
    // coordinates at their lower bounds, uniquely; f=1e-8*(-1e6)=-0.01.
    for call in [
        "linprog([1e-8;1],[],[],[],[],[-1e6;0],[1e6;3])",
        "intlinprog([1e-8;1],1:2,[],[],[],[],[-1e6;0],[1e6;3])",
        "intlinprog([1e-8;1],2,[],[],[],[],[-1e6;0],[1e6;3])",
    ] {
        run(&mut e, &format!("[x,f,flag,out]={call};"));
        assert_eq!(scalar(&e, "flag"), 1., "{call}");
        // The default absolute duality-gap tolerance is 1e-6; a 1e-8
        // objective coefficient permits a coordinate error of up to 100.
        close(&values(&e, "x"), &[-1e6, 0.], 100.);
        close(&values(&e, "f"), &[-0.01], 1e-6);
        run(&mut e, "cv=out.constrviolation;");
        assert!(scalar(&e, "cv") <= 1e-6);
    }
    run(
        &mut e,
        "op=optimoptions('linprog','OptimalityTolerance',1e-11,'ConstraintTolerance',1e-9); [x,f,flag,out,L]=linprog([1e-8;1],[],[],[],[],[-1e6;0],[1e6;3],op);",
    );
    assert_eq!(
        scalar(&e, "flag"),
        1.,
        "tight LP x={:?}, f={:?}",
        values(&e, "x"),
        values(&e, "f")
    );
    run(
        &mut e,
        "station=[1e-8;1]-L.lower+L.upper; dual=[-1e6;0]'*L.lower-[1e6;3]'*L.upper; gap=f-dual;",
    );
    close(&values(&e, "x"), &[-1e6, 0.], 0.001);
    close(&values(&e, "f"), &[-0.01], 1e-11);
    assert!(values(&e, "station").iter().all(|v| v.abs() <= 3e-11));
    assert!(scalar(&e, "gap").abs() <= 1e-11);
}

#[test]
fn integer_one_node_retains_feasible_rounding() {
    // Exact enumeration: both 0 and 1 satisfy x<=2 and the binary bounds;
    // each has objective zero. No tie-dependent coordinate is prescribed.
    check_one_node_integer_rounding("0,1,1,2,[],[],0,1", 1, 2.);
}

#[test]
fn integer_one_node_retains_feasible_rounding_two_variables() {
    // Exact feasible set: (0,0), (1,0), (0,1), all with objective zero.
    check_one_node_integer_rounding("[0;0],[1 2],[1 1],1,[],[],[0;0],[1;1]", 2, 1.);
}

fn check_one_node_integer_rounding(args: &str, n: usize, rhs: f64) {
    let mut e = setup();
    run(
        &mut e,
        &format!(
            "[x,f,flag,out]=intlinprog({args},struct('MaxNodes',1)); nodes=out.numnodes; ag=out.absolutegap; rg=out.relativegap; cv=out.constrviolation; msg=out.message;"
        ),
    );
    let x = values(&e, "x");
    let flag = scalar(&e, "flag");
    let absolute = scalar(&e, "ag");
    let relative = scalar(&e, "rg");
    println!(
        "{args}: x={x:?}, f={:?}, flag={flag}, nodes={}, absolute={absolute}, relative={relative}, message={}",
        values(&e, "f"),
        scalar(&e, "nodes"),
        text(&e, "msg")
    );
    assert_eq!(x.len(), n, "a verified integer incumbent must be retained");
    assert!(x.iter().all(|v| v.is_finite() && v.fract() == 0.));
    assert!(x.iter().all(|v| (0. ..=1.).contains(v)));
    assert!(x.iter().sum::<f64>() <= rhs);
    assert_eq!(scalar(&e, "f"), 0.);
    assert_eq!(scalar(&e, "nodes"), 1.);
    assert_eq!(scalar(&e, "cv"), 0.);
    assert!(absolute.is_finite() && absolute >= 0.);
    assert_eq!(
        relative, absolute,
        "zero objective uses gap denominator one"
    );
    // Both default gap tolerances are zero. Feasibility alone
    // does not prove optimality while the original children remain open.
    assert_eq!(flag, if absolute == 0. { 1. } else { 2. });
}

#[test]
fn integer_later_worse_rounding_preserves_better_incumbent() {
    let mut e = setup();
    // Enumerate the binary feasible set: (0,0), (1,0), (0,1) have
    // costs 0, -1, -0.7. The unique best integer point is (1,0).
    // Root LP: (1,0.25) rounds to (1,0), but must branch on x2.
    // With an incumbent, best-bound visits the first pushed child x2>=1:
    // its LP point (0.4,1) costs -1.1 < -1, so cannot be pruned, yet
    // rounds to the feasible WORSE point (0,1), costing -0.7.
    for nodes in [1, 2] {
        run(
            &mut e,
            &format!(
                "[x,f,flag,out]=intlinprog([-1;-0.7],[1 2],[1 0.8],1.2,[],[],[0;0],[1;1],struct('MaxNodes',{nodes})); nodes=out.numnodes; ag=out.absolutegap; cv=out.constrviolation;"
            ),
        );
        println!(
            "budget={nodes}: x={:?}, f={}, flag={}, nodes={}, gap={}",
            values(&e, "x"),
            scalar(&e, "f"),
            scalar(&e, "flag"),
            scalar(&e, "nodes"),
            scalar(&e, "ag")
        );
        assert_eq!(values(&e, "x"), [1., 0.]);
        assert_eq!(scalar(&e, "f"), -1.);
        assert_eq!(scalar(&e, "nodes"), f64::from(nodes));
        assert_eq!(scalar(&e, "flag"), 2.);
        assert_eq!(scalar(&e, "cv"), 0.);
        assert!(scalar(&e, "ag").is_finite() && scalar(&e, "ag") > 0.);
    }
}

#[test]
fn integer_rounding_must_preserve_constraint_tolerance() {
    let mut e = setup();
    // Enumerating {0,1,2}: x=0 is the unique exactly feasible optimum.
    // x=1 is accepted only in the second case, within ConstraintTolerance.
    for (rhs, expected) in [("0.999999", 0.), ("1-5e-7", 1.)] {
        run(
            &mut e,
            &format!("[x,f,flag,out]=intlinprog(-1,1,1,{rhs},[],[],0,2); cv=out.constrviolation;"),
        );
        run(&mut e, "nodes=out.numnodes; msg=out.message;");
        assert_eq!(
            scalar(&e, "flag"),
            1.,
            "rhs={rhs}, x={:?}, nodes={}, msg={}",
            values(&e, "x"),
            scalar(&e, "nodes"),
            text(&e, "msg")
        );
        close(&values(&e, "x"), &[expected], 0.);
        close(&values(&e, "f"), &[-expected], 0.);
        assert!(scalar(&e, "cv") <= 1e-6);
    }
}

#[test]
fn sqp_positive_elastic_slack_reports_restoration_failure() {
    let mut e = setup();
    // For every real x, x^2+1>=1: the equality has no feasible point.
    run(
        &mut e,
        "[x,f,flag,out,L]=fmincon(@(x) (x-2)^2,0,[],[],[],[],-2,2,@(x) deal([],x^2+1));",
    );
    assert_eq!(scalar(&e, "flag"), -2.);
    empty_solution(&mut e, &["x", "f"], true);
}

#[test]
fn optimoptions_help_lists_integer_search_options() {
    let help = include_str!("../assets/help/functions.md");
    let page = help
        .split("## optimoptions\n")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    for name in [
        "MaxNodes",
        "IntegerTolerance",
        "RelativeGapTolerance",
        "AbsoluteGapTolerance",
    ] {
        assert!(page.contains(name), "optimoptions help missing {name}");
    }
}

#[test]
fn empty_problems_report_no_variables_without_panicking() {
    let mut e = setup();
    for (name, calls) in [
        (
            "linprog",
            vec![
                "linprog([])",
                "linprog(zeros(0,1),[],[])",
                "linprog(struct('f',[]))",
            ],
        ),
        (
            "quadprog",
            vec![
                "quadprog([],[])",
                "quadprog(zeros(0,0),zeros(0,1),[],[])",
                "quadprog(struct('H',[],'f',[]))",
            ],
        ),
        (
            "lsqlin",
            vec![
                "lsqlin([],[])",
                "lsqlin(zeros(2,0),ones(2,1),[],[])",
                "lsqlin(struct('C',[],'d',[]))",
            ],
        ),
        (
            "fmincon",
            vec![
                "fmincon(@(x) 0,[],[],[])",
                "fmincon(@(x) 0,zeros(0,1),[],[])",
                "fmincon(struct('objective',@(x) 0,'x0',[]))",
            ],
        ),
        (
            "intlinprog",
            vec![
                "intlinprog([],[])",
                "intlinprog(zeros(0,1),[],[],[])",
                "intlinprog(struct('f',[],'intcon',[]))",
            ],
        ),
    ] {
        for call in calls {
            for source in [call.to_string(), format!("[x,v,flag]={call};")] {
                let err = e.execute(&source).expect_err(&source);
                assert!(
                    err.contains(&format!("{name}: the problem has no variables")),
                    "{source}: {err}"
                );
                assert!(!err.contains("exceeds the limit"), "{err}");
            }
        }
    }
    // Empty f still denotes feasibility when other inputs supply variables.
    run(&mut e, "[x,f,flag]=quadprog(eye(2),[]);");
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[0., 0.], 1e-6);
}

#[test]
fn sqp_backtracks_nonfinite_and_complex_objective_trials() {
    let mut e = setup();
    run(
        &mut e,
        "[x,f,flag,out]=fmincon(@(x) log(x)+x,1,[],[],[],[],0,2); fc=out.funcCount;",
    );
    // log(x)+x decreases without a finite minimum as x -> 0+. A valid
    // starting point must survive rejected trials at zero, without claiming
    // a finite global minimum. Check a finite, improving iterate.
    assert!(scalar(&e, "x") > 0. && scalar(&e, "x") < 1.);
    assert!(scalar(&e, "f").is_finite() && scalar(&e, "f") < 1.);
    assert!(scalar(&e, "fc") <= 3000.);
    for lb in [0, -2] {
        // Nonnegative square has unique zero where log(x)+x=0; derivative
        // 1/x+1 is positive. Bounds force a nonfinite or complex first trial.
        run(
            &mut e,
            &format!(
                "[x,f,flag,out,lambda,g]=fmincon(@(x) (log(x)+x)^2,1,[],[],[],[],{lb},2); cv=out.constrviolation;"
            ),
        );
        assert!(scalar(&e, "flag") > 0., "lower bound {lb}");
        assert!(scalar(&e, "x") > 0. && scalar(&e, "x") < 1.);
        close(&values(&e, "f"), &[0.], 1e-8);
        close(&values(&e, "g"), &[0.], 1e-5);
        assert!(scalar(&e, "cv") <= 1e-6);
    }
    for x0 in [0, -1] {
        let source = format!("fmincon(@(x) log(x)+x,{x0},[],[],[],[],-2,2)");
        let err = e.execute(&source).expect_err(&source);
        assert!(
            err.contains("objective at the starting point must be finite and real"),
            "{err}"
        );
    }
    run(
        &mut e,
        "op=optimoptions('fmincon','MaxFunctionEvaluations',3,'Display','off'); [x,f,flag,out]=fmincon(@(x) (log(x)+x)^2,1,[],[],[],[],-2,2,[],op); fc=out.funcCount;",
    );
    assert_eq!(scalar(&e, "flag"), 0.);
    assert_eq!(scalar(&e, "fc"), 3.);
    close(&values(&e, "x"), &[1.], 0.);
}

#[test]
fn sqp_finite_differences_retry_domains_and_preserve_the_budget() {
    let mut e = setup();
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    for (name, invalid) in [
        ("window", "abs(x)>1e-10"),
        ("left", "x>0"),
        ("right", "x<0"),
    ] {
        std::fs::write(
            dir.path().join(format!("domain_{name}.m")),
            format!("function f=domain_{name}(x)\nif {invalid}\nf=Inf;\nelse\nf=3*x;\nend\nend\n"),
        )
        .unwrap();
    }
    e.set_function_directory(dir.path()).unwrap();
    std::fs::write(
        dir.path().join("domain_complex.m"),
        "function f=domain_complex(x)\nif x<0\nf=sqrt(-1);\nelse\nf=3*x;\nend\nend\n",
    )
    .unwrap();
    for diff in ["forward", "central"] {
        for name in ["window", "left", "right", "complex"] {
            run(
                &mut e,
                &format!(
                    "op=optimoptions('fmincon','MaxIterations',0,'FiniteDifferenceType','{diff}','Display','off'); [x,f,flag,out,lambda,g]=fmincon(@domain_{name},0,[],[],[],[],-1,1,[],op); fc=out.funcCount;"
                ),
            );
            // The finite domain is linear: both one-sided and reduced central
            // differences must recover the exact derivative 3.
            close(&values(&e, "g"), &[3.], 1e-12);
            assert_eq!(scalar(&e, "flag"), 0.);
            if name == "window" {
                assert!(scalar(&e, "fc") > 3.);
            }
        }
    }
    run(
        &mut e,
        "op.MaxFunctionEvaluations=3; [x,f,flag,out]=fmincon(@domain_window,0,[],[],[],[],-1,1,[],op); fc=out.funcCount;",
    );
    assert_eq!(scalar(&e, "flag"), 0.);
    assert_eq!(scalar(&e, "fc"), 3.);
    // Finite start, no finite neighbouring points: name the failed probes.
    std::fs::write(
        dir.path().join("isolated.m"),
        "function f=isolated(x)\nif x==0\nf=0;\nelse\nf=NaN;\nend\nend\n",
    )
    .unwrap();
    let err = e.execute("fmincon(@isolated,0,[],[])").unwrap_err();
    assert!(err.contains("finite-difference trial points"), "{err}");
    assert!(!err.contains("starting point"), "{err}");
    // Callback programming errors remain errors, rather than domain retries.
    let err = e.execute("fmincon(@(x) [x;x],1,[],[])").unwrap_err();
    assert!(err.contains("scalar"), "{err}");
}

#[test]
fn integer_search_prunes_both_infeasible_relaxations() {
    let mut e = setup();
    // x2 <= 0 and x2 >= 1 contradict, while x1 is a dual recession direction.
    run(&mut e, "[x,f,flag]=linprog([-1;0],[0 1;0 -1],[0;-1]);");
    assert_eq!(scalar(&e, "flag"), -5.);
    run(
        &mut e,
        "[x,f,flag,out]=intlinprog([-1;0],1,[0 1;0 -1],[0;-1]); nodes=out.numnodes;",
    );
    assert_eq!(scalar(&e, "flag"), -2.);
    assert_eq!(scalar(&e, "nodes"), 1.);
    empty_solution(&mut e, &["x", "f"], false);
}

#[test]
fn tall_least_squares_counts_user_rows_once_and_preserves_residuals() {
    let mut e = setup();
    for m in [1200, 4000] {
        run(
            &mut e,
            &format!(
                "[x,rn,res,flag,out,lambda]=lsqlin(ones({m},2),ones({m},1)); sx=sum(x); kkt=out.firstorderopt;"
            ),
        );
        assert_eq!(scalar(&e, "flag"), 1.);
        // Nonunique x, unique fitted value x1+x2=1.
        close(&values(&e, "sx"), &[1.], 1e-6);
        close(&values(&e, "rn"), &[0.], 1e-8);
        close(&values(&e, "res"), &vec![0.; m], 1e-6);
        // C' sums m residuals: scale the default-tolerance KKT check.
        assert!(scalar(&e, "kkt") < m as f64 * 1e-6);
    }
    // Orthogonal columns and noise give x=[2.5;3], resnorm=450 and lower
    // multiplier 600 by the least-squares KKT equations. The QR residual
    // orthogonal to C contributes 150 and must remain in resnorm/residual.
    run(
        &mut e,
        "t=(1:1200)'; C=[ones(1200,1) (-1).^t]; d=2+3*C(:,2)+0.5*cos(pi*t/2); [x,rn,res,flag,out,lambda]=lsqlin(C,d,[0 1],4,[0 1],3,[2.5;-Inf],[],[],o); station=C'*res+[0;1]*lambda.ineqlin+[0;1]*lambda.eqlin-lambda.lower+lambda.upper; low=lambda.lower; cv=out.constrviolation;",
    );
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[2.5, 3.], 1e-6);
    close(&values(&e, "rn"), &[450.], 1e-4);
    close(&values(&e, "low"), &[600., 0.], 1e-4);
    close(&values(&e, "station"), &[0., 0.], 1e-5);
    assert!(scalar(&e, "cv") < 1e-6);
    for source in [
        "lsqlin(ones(4001,2),ones(4001,1))",
        "lsqlin(ones(4000,2),ones(4000,1),[1 0],2)",
    ] {
        let err = e.execute(source).expect_err(source);
        assert!(err.contains("limit"), "{err}");
    }
}

#[test]
fn constraint_gradient_errors_name_the_constraint_option() {
    let mut e = setup();
    for source in [
        "optimoptions('fmincon','SpecifyConstraintGradient','invalid')",
        "optimoptions('fmincon','SpecifyConstraintGradient',[1 2])",
        "optimoptions('fmincon','GradConstr','invalid')",
        "fmincon(@(x) x^2,1,[],[],[],[],[],[],[],struct('SpecifyConstraintGradient','invalid'))",
    ] {
        let err = e.execute(source).expect_err(source);
        assert!(err.contains("SpecifyConstraintGradient"), "{source}: {err}");
        assert!(!err.contains("SpecifyObjectiveGradient"), "{source}: {err}");
    }
}

#[test]
fn integer_cleanup_preserves_exact_integrality_at_unrounded_bounds() {
    let mut e = setup();
    for (cost, lb, ub, expected) in [
        (1., -1e-8, 2., 0.),
        (-1., 0., 1. + 1e-8, 1.),
        (1., 0.2, 2.8, 1.),
        (-1., 0.2, 2.8, 2.),
    ] {
        run(
            &mut e,
            &format!(
                "[x,f,flag,out]=intlinprog({cost},1,[],[],[],[],{lb},{ub}); cv=out.constrviolation;"
            ),
        );
        assert_eq!(scalar(&e, "flag"), 1.);
        assert_eq!(scalar(&e, "x"), expected);
        assert_eq!(scalar(&e, "x").fract(), 0.);
        assert_eq!(scalar(&e, "f"), cost * expected);
        assert!(scalar(&e, "cv") <= 1e-6);
    }
    // A mixed point retains continuous bound snapping and exact integer entries.
    run(
        &mut e,
        "[x,f,flag]=intlinprog([1;-1],1,[],[],[],[],[-1e-8;0],[2;1+1e-8]);",
    );
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[0., 1. + 1e-8], 0.);
}

fn check_scaled_cleanup(call: &str, integer: bool, equality: bool) {
    let mut e = Engine::default();
    run(
        &mut e,
        &format!("[x,f,flag,out]={call}; cv=out.constrviolation;"),
    );
    let x = values(&e, "x");
    let flag = scalar(&e, "flag");
    let cv = scalar(&e, "cv");
    assert_eq!(flag, 1., "{call}: x={x:?}, flag={flag}, cv={cv}");
    if integer {
        assert_eq!(x[0], 0., "integer coordinate must stay exact");
        run(&mut e, "nodes=out.numnodes;");
        assert_eq!(scalar(&e, "nodes"), 1., "no spurious integer branching");
    }
    let continuous = x[usize::from(integer)];
    // Exact arithmetic: 1000*x=5e-4 gives x=5e-7; minimizing x subject
    // to -2500*x<=-1e-3 gives x=4e-7. Snapping either to zero is infeasible.
    let violation = if equality {
        (1000. * continuous - 5e-4).abs()
    } else {
        (1e-3 - 2500. * continuous).max(0.)
    }
    .max((-continuous).max(continuous - 1.));
    assert!(
        violation <= 1e-6,
        "{call}: x={x:?}, violation={violation}, cv={cv}"
    );
    assert!(
        (cv - violation).abs() <= 1e-12,
        "reported violation must describe x"
    );
    assert_eq!(scalar(&e, "f"), continuous, "objective must describe x");
}

#[test]
fn continuous_cleanup_preserves_scaled_equality() {
    check_scaled_cleanup("linprog(1,[],[],1000,5e-4,0,1)", false, true);
}

#[test]
fn continuous_cleanup_preserves_scaled_inequality() {
    check_scaled_cleanup("linprog(1,-2500,-1e-3,[],[],0,1)", false, false);
}

#[test]
fn integer_cleanup_preserves_scaled_equality_incumbent() {
    check_scaled_cleanup(
        "intlinprog([0;1],1,[],[],[0 1000],5e-4,[0;0],[0;1])",
        true,
        true,
    );
}

#[test]
fn integer_cleanup_preserves_scaled_inequality_incumbent() {
    check_scaled_cleanup(
        "intlinprog([0;1],1,[0 -2500],-1e-3,[],[],[0;0],[0;1])",
        true,
        false,
    );
}

#[test]
fn constrained_options_reject_inapplicable_fields() {
    let mut e = Engine::default();
    for (solver, args) in [
        ("linprog", "1,[],[],[],[],0,1"),
        ("quadprog", "2,0,[],[],[],[],0,1,[]"),
        ("lsqlin", "1,0,[],[],[],[],0,1,[]"),
        ("fmincon", "@(x)x^2,0,[],[],[],[],0,1,[]"),
        ("intlinprog", "1,1,[],[],[],[],0,1"),
    ] {
        let outputs = if solver == "lsqlin" {
            "x,f,residual,flag"
        } else {
            "x,f,flag"
        };
        for (field, value, applicable) in [
            ("MaxNodes", "1", solver == "intlinprog"),
            ("IntegerTolerance", "1e-6", solver == "intlinprog"),
            ("RelativeGapTolerance", "0", solver == "intlinprog"),
            ("AbsoluteGapTolerance", "0", solver == "intlinprog"),
            ("SpecifyConstraintGradient", "false", solver == "fmincon"),
            ("GradConstr", "false", solver == "fmincon"),
            ("ConstraintTolerance", "1e-6", true),
            ("TolCon", "1e-6", true),
        ] {
            for option in [
                format!("struct('{field}',{value})"),
                format!("optimoptions('{solver}','{field}',{value})"),
                // Untyped struct updates must be checked when the solver consumes them.
                format!("optimoptions(struct(),'{field}',{value})"),
            ] {
                let source = format!("[{outputs}]={solver}({args},{option});");
                if applicable {
                    run(&mut e, &source);
                    assert_eq!(scalar(&e, "flag"), 1., "{source}");
                    assert!((-1e-6..=1. + 1e-6).contains(&scalar(&e, "x")), "{source}");
                } else {
                    let err = e.execute(&source).expect_err(&source);
                    assert!(
                        err.contains(field)
                            || (field == "GradConstr" && err.contains("SpecifyConstraintGradient")),
                        "{source}: {err}"
                    );
                    assert!(err.contains("not implemented"), "{source}: {err}");
                }
            }
            if !applicable {
                for named in [format!("'{solver}'"), format!("@{solver}")] {
                    let source = format!("optimoptions({named},'{field}',{value})");
                    let err = e.execute(&source).expect_err(&source);
                    assert!(
                        err.contains(field)
                            || (field == "GradConstr" && err.contains("SpecifyConstraintGradient")),
                        "{source}: {err}"
                    );
                    assert!(err.contains("not implemented"), "{source}: {err}");
                }
            }
            // Empty fields keep defaults, including when inapplicable. A shared
            // supported option still controls execution (zero iteration budget).
            for option in [
                format!("struct('{field}',[],'MaxIterations',0,'Display','off')"),
                format!("optimoptions('{solver}','{field}',[],'MaxIterations',0,'Display','off')"),
            ] {
                run(
                    &mut e,
                    &format!("[{outputs},out]={solver}({args},{option}); it=out.iterations;"),
                );
                assert_eq!(scalar(&e, "flag"), 0.);
                // intlinprog counts its attempted root node as an iteration.
                assert_eq!(
                    scalar(&e, "it"),
                    if solver == "intlinprog" { 1. } else { 0. }
                );
            }
        }
    }
}

#[test]
fn box_lp_limits_count_user_arrays_instead_of_generated_bounds() {
    let mut e = setup();
    for n in [1500, 2000] {
        run(
            &mut e,
            &format!(
                "[x,f,flag,out,lambda]=linprog(ones({n},1),[],[],[],[],zeros({n},1),ones({n},1)); cv=out.constrviolation; station=ones({n},1)-lambda.lower+lambda.upper;"
            ),
        );
        assert_eq!(scalar(&e, "flag"), 1.);
        close(&values(&e, "x"), &vec![0.; n], 1e-6);
        close(&values(&e, "f"), &[0.], 1e-6);
        close(&values(&e, "station"), &vec![0.; n], 1e-6);
        assert!(scalar(&e, "cv") <= 1e-6);
    }
    // Endpoint selection and bound multipliers for both cost signs and a
    // zero-cost coordinate, including a fixed variable.
    run(
        &mut e,
        "[x,f,flag,out,L]=linprog([2;-3;0;4],[],[],[],[],[-2;1;5;7],[3;4;6;7]); station=[2;-3;0;4]-L.lower+L.upper;",
    );
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[-2., 4., 5., 7.], 0.);
    close(&values(&e, "f"), &[12.], 0.);
    close(&values(&e, "station"), &[0.; 4], 0.);
    for (source, diagnostic) in [
        ("linprog(ones(2001,1))", "limit"),
        ("linprog(1,ones(4001,1),ones(4001,1))", "limit"),
        (
            "linprog(ones(2000,1),ones(2001,2000),ones(2001,1))",
            "A,b is too large",
        ),
    ] {
        let err = e.execute(source).expect_err(source);
        assert!(err.contains(diagnostic), "{source}: {err}");
    }
}

#[test]
fn square_and_wide_lsq_limits_count_user_arrays() {
    let mut e = setup();
    run(&mut e, "[x,rn,res,flag]=lsqlin(zeros(0,2),zeros(0,1));");
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "rn"), &[0.], 0.);
    assert!(values(&e, "res").is_empty());
    run(
        &mut e,
        "[x,rn,res,flag,out]=lsqlin(eye(700),ones(700,1)); kkt=out.firstorderopt; cv=out.constrviolation;",
    );
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &vec![1.; 700], 1e-5);
    close(&values(&e, "res"), &vec![0.; 700], 1e-6);
    close(&values(&e, "rn"), &[0.], 1e-8);
    assert!(scalar(&e, "kkt") <= 1e-6);
    assert!(scalar(&e, "cv") <= 1e-6);
    for n in [1999, 2000] {
        run(
            &mut e,
            &format!(
                "[x,rn,res,flag,out]=lsqlin(ones(1,{n}),1); sx=sum(x); kkt=out.firstorderopt;"
            ),
        );
        assert_eq!(scalar(&e, "flag"), 1.);
        // The fitted value is unique; the underdetermined x need not be.
        close(&values(&e, "sx"), &[1.], 1e-6);
        close(&values(&e, "res"), &[0.], 1e-6);
        close(&values(&e, "rn"), &[0.], 1e-8);
        assert!(scalar(&e, "kkt") <= 1e-6);
    }
    // Exercise the constrained augmented route too: 2000 original variables
    // plus one residual. Fixed-column presolve leaves the unique x(2000)=1.
    run(
        &mut e,
        "lb=zeros(2000,1); ub=lb; ub(2000)=2; [x,rn,res,flag,out,L]=lsqlin(ones(1,2000),1,[],[],[],[],lb,ub); last=x(2000); sx=sum(x); kkt=out.firstorderopt; cv=out.constrviolation;",
    );
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "last"), &[1.], 1e-5);
    close(&values(&e, "sx"), &[1.], 1e-5);
    close(&values(&e, "rn"), &[0.], 1e-8);
    assert!(scalar(&e, "kkt") <= 1e-6);
    assert!(scalar(&e, "cv") <= 1e-6);
    assert!(
        e.execute("lsqlin(ones(1,2001),1)")
            .unwrap_err()
            .contains("limit")
    );
}

// Original-coordinate residuals, from the exact equations x=rhs and x(2)=rhs.
// These cover the equality-only LP, primal-dual QP and bounded MILP relaxation.
fn check_large_equality(solver: &str, rhs: f64, tol: f64, explicit: bool) {
    let mut e = Engine::default();
    let option = if explicit {
        format!(",struct('ConstraintTolerance',{tol})")
    } else {
        String::new()
    };
    let call = match solver {
        "linprog" => format!("linprog(1,[],[],1,{rhs},[],[]{option})"),
        "quadprog" => format!("quadprog(2,0,[],[],1,{rhs},[],[],[]{option})"),
        _ => format!(
            "intlinprog([0;1],1,[],[],[0 1],{rhs},[0;0],[0;{}]{option})",
            2. * rhs
        ),
    };
    run(
        &mut e,
        &format!("[x,f,flag,out]={call}; cv=out.constrviolation;"),
    );
    let x = values(&e, "x");
    let flag = scalar(&e, "flag");
    let cv = scalar(&e, "cv");
    eprintln!("{call}: x={x:?}, flag={flag}, cv={cv}");
    assert_eq!(flag, 1., "{call}: x={x:?}, flag={flag}, cv={cv}");
    assert_eq!(x.len(), if solver == "intlinprog" { 2 } else { 1 });
    if solver == "intlinprog" {
        assert_eq!(x[0], 0., "integer coordinate stays exact");
    }
    let v = (x[x.len() - 1] - rhs).abs();
    assert!(v <= tol, "{call}: x={x:?}, residual={v}, tol={tol}");
    assert!(
        (v - cv).abs() <= 1e-12,
        "reported {cv} differs from residual {v}"
    );
    let expected = if solver == "quadprog" {
        x[0] * x[0]
    } else {
        x[x.len() - 1]
    };
    assert_eq!(scalar(&e, "f"), expected);
}
macro_rules! large_equality_test {
    ($name:ident,$solver:literal,$rhs:expr,$tol:expr,$explicit:expr) => {
        #[test]
        fn $name() {
            check_large_equality($solver, $rhs, $tol, $explicit);
        }
    };
}
large_equality_test!(absolute_equality_lp, "linprog", 1e6, 1e-6, false);
large_equality_test!(absolute_equality_qp, "quadprog", 1e6, 1e-6, false);
large_equality_test!(absolute_equality_milp, "intlinprog", 1e6, 1e-6, false);
large_equality_test!(
    absolute_equality_lp_second_scale,
    "linprog",
    3e7,
    1e-6,
    false
);
large_equality_test!(
    absolute_equality_qp_second_scale,
    "quadprog",
    3e7,
    1e-6,
    false
);
large_equality_test!(
    absolute_equality_milp_second_scale,
    "intlinprog",
    3e7,
    1e-6,
    false
);
large_equality_test!(
    absolute_equality_lp_user_tolerance,
    "linprog",
    2e5,
    1e-8,
    true
);
large_equality_test!(
    absolute_equality_qp_user_tolerance,
    "quadprog",
    2e5,
    1e-8,
    true
);
large_equality_test!(
    absolute_equality_milp_user_tolerance,
    "intlinprog",
    2e5,
    1e-8,
    true
);

fn check_sqp_endpoint(analytic: bool) {
    let mut e = Engine::default();
    let (fun, options) = if analytic {
        (
            "@(x) deal(log(x)+2*x,1/x+2)",
            ",[],struct('SpecifyObjectiveGradient',true)",
        )
    } else {
        ("@(x) log(x)+x", "")
    };
    run(
        &mut e,
        &format!(
            "[x,f,flag,out,L]=fmincon({fun},1,[],[],[],[],0,2{options}); opt=out.firstorderopt; comp=max(abs([L.lower*x;L.upper*(2-x)])); fc=out.funcCount;"
        ),
    );
    let x = scalar(&e, "x");
    let flag = scalar(&e, "flag");
    let comp = scalar(&e, "comp");
    let opt = scalar(&e, "opt");
    eprintln!(
        "analytic={analytic}: x={x}, flag={flag}, optimality={opt}, complementarity={comp}, evaluations={}",
        scalar(&e, "fc")
    );
    // log(x)+a*x has derivative 1/x+a>0 and no finite minimum on (0,2].
    // The nonfinite endpoint is not an optimum; this tests stopping, not classification.
    assert_ne!(
        flag, 1.,
        "false current-point KKT success: x={x}, opt={opt}, comp={comp}"
    );
    assert!(x > 0. && x < 1. && scalar(&e, "f").is_finite());
    assert!(
        opt + 1e-10 * comp.max(1.) >= comp,
        "firstorderopt must include current complementarity"
    );
    assert!(scalar(&e, "fc") <= 3000.);
}
#[test]
fn sqp_current_complementarity_log() {
    check_sqp_endpoint(false);
}
#[test]
fn sqp_current_complementarity_analytic_log() {
    check_sqp_endpoint(true);
}

#[test]
fn sqp_current_kkt_quadratic_controls() {
    let mut e = Engine::default();
    // f=(x-1)^2: interior optimum 1, zero duals. f=(x+1)^2: lower-active
    // optimum 0, lower multiplier 2 from 2*(x+1)-lambda.lower=0.
    for (fun, expected, lower) in [("@(x) (x-1)^2", 1., 0.), ("@(x) (x+1)^2", 0., 2.)] {
        run(
            &mut e,
            &format!(
                "[x,f,flag,out,L,g]=fmincon({fun},0.5,[],[],[],[],0,2); lo=L.lower; hi=L.upper; opt=out.firstorderopt; station=g-lo+hi; comp=max(abs([lo*x;hi*(2-x)])); cv=out.constrviolation;"
            ),
        );
        assert_eq!(scalar(&e, "flag"), 1., "{fun}");
        close(&values(&e, "x"), &[expected], 1e-6);
        close(&values(&e, "lo"), &[lower], 1e-6);
        close(&values(&e, "hi"), &[0.], 1e-6);
        assert!(scalar(&e, "lo") >= 0. && scalar(&e, "hi") >= 0.);
        assert!(scalar(&e, "station").abs() <= 1e-6);
        assert!(scalar(&e, "comp") <= 1e-6 && scalar(&e, "opt") <= 1e-6);
        assert!(scalar(&e, "cv") <= 1e-6);
    }
}

#[test]
fn sqp_current_kkt_nonlinear_active_control() {
    let mut e = Engine::default();
    // KKT for min (x-2)^2 with x^2<=1: x=1, lambda=1.
    run(
        &mut e,
        "o=struct('SpecifyObjectiveGradient',true,'SpecifyConstraintGradient',true); [x,f,flag,out,L,g]=fmincon(@(x) deal((x-2)^2,2*(x-2)),0.5,[],[],[],[],[],[],@(x) deal(x^2-1,[],2*x,[]),o); lam=L.ineqnonlin; opt=out.firstorderopt; comp=lam*(x^2-1); station=g+2*x*lam; cv=out.constrviolation;",
    );
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[1.], 1e-6);
    close(&values(&e, "lam"), &[1.], 1e-6);
    assert!(scalar(&e, "lam") >= 0.);
    assert!(scalar(&e, "comp").abs() <= 1e-6);
    assert!(scalar(&e, "station").abs() <= 1e-6);
    assert!(scalar(&e, "opt") <= 1e-6 && scalar(&e, "cv") <= 1e-6);
}

#[test]
fn sqp_current_kkt_keeps_limit_statuses() {
    let mut e = Engine::default();
    for (options, expected) in [
        ("struct('MaxIterations',1)", 0.),
        ("struct('StepTolerance',1)", 2.),
    ] {
        run(
            &mut e,
            &format!(
                "[x,f,flag,out]=fmincon(@(x) log(x)+x,1,[],[],[],[],0,2,[],{options}); opt=out.firstorderopt;"
            ),
        );
        assert_eq!(scalar(&e, "flag"), expected);
        assert!(scalar(&e, "x") > 0. && scalar(&e, "f").is_finite());
        assert!(scalar(&e, "opt") > 1e-6);
    }
}

#[test]
fn qp_tight_optimality_preserves_interior_point() {
    // Strict convexity and x-0.9995=0 give the unique interior minimizer.
    let mut e = setup();
    run(
        &mut e,
        "o=struct('ConstraintTolerance',1e-3,'OptimalityTolerance',1e-12); [x,f,flag,out]=quadprog(1,-0.9995,[],[],[],[],-2,1,[],o); fo=out.firstorderopt; cv=out.constrviolation;",
    );
    println!(
        "QP x={:?}, f={:?}, flag={}, firstorderopt={}, cv={}",
        values(&e, "x"),
        values(&e, "f"),
        scalar(&e, "flag"),
        scalar(&e, "fo"),
        scalar(&e, "cv")
    );
    close(&values(&e, "x"), &[0.9995], 1e-7);
    close(&values(&e, "f"), &[-0.5 * 0.9995 * 0.9995], 1e-12);
    assert_eq!(scalar(&e, "flag"), 1.);
    assert!(scalar(&e, "fo") <= 2e-12);
    assert_eq!(scalar(&e, "cv"), 0.);
}

#[test]
fn ls_tight_optimality_preserves_interior_point() {
    // Full-rank scalar least squares: residual x-0.9995 uniquely vanishes.
    let mut e = setup();
    run(
        &mut e,
        "o=struct('ConstraintTolerance',1e-3,'OptimalityTolerance',1e-12); [x,rn,residual,flag,out]=lsqlin(1,0.9995,[],[],[],[],-2,1,[],o); fo=out.firstorderopt; cv=out.constrviolation;",
    );
    println!(
        "LS x={:?}, resnorm={:?}, residual={:?}, flag={}, firstorderopt={}, cv={}",
        values(&e, "x"),
        values(&e, "rn"),
        values(&e, "residual"),
        scalar(&e, "flag"),
        scalar(&e, "fo"),
        scalar(&e, "cv")
    );
    close(&values(&e, "x"), &[0.9995], 1e-7);
    close(&values(&e, "residual"), &[scalar(&e, "x") - 0.9995], 1e-15);
    close(&values(&e, "rn"), &[scalar(&e, "residual").powi(2)], 1e-25);
    assert!(scalar(&e, "rn") <= 1e-14);
    assert_eq!(scalar(&e, "flag"), 1.);
    assert!(scalar(&e, "fo") <= 2e-12);
    assert_eq!(scalar(&e, "cv"), 0.);
}

fn run_out(e: &mut Engine, s: &str) -> Vec<String> {
    match e.execute(s) {
        Ok(out) => out.output,
        Err(err) => panic!("{s}\n{err}"),
    }
}

fn fails(e: &mut Engine, s: &str) -> String {
    match e.execute(s) {
        Ok(_) => panic!("{s} should fail"),
        Err(err) => err.to_string(),
    }
}

#[test]
fn fmincon_interior_point_and_hessian_handle_warn_and_run_sqp_without_calling_hessian() {
    let mut e = Engine::default();
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    callbacks(&mut e, dir.path());

    // 1. Reproduce & verify constructor + lastwarn separately from solve on a small bounded quadratic:
    //    min 0.5*((x1-2)^2 + 4*(x2+1)^2) s.t. 0 <= x <= [1.5; 0] -> optimum at [1.5; -1].
    //    HessianFcn handle would error if invoked under the SQP fallback.
    let ctor_out = run_out(
        &mut e,
        "lastwarn('',''); \
         o = optimoptions('fmincon','Display','off','Algorithm','interior-point',\
         'HessianFcn',@(x,lambda) error('hessian_should_not_be_called')); \
         [lw_msg, lw_id] = lastwarn;",
    );
    assert_eq!(ctor_out.len(), 2, "{ctor_out:?}");
    assert!(
        ctor_out[0].contains("fmincon") && ctor_out[0].contains("Algorithm"),
        "{ctor_out:?}"
    );
    assert!(
        ctor_out[1].contains("fmincon") && ctor_out[1].contains("HessianFcn"),
        "{ctor_out:?}"
    );
    assert!(text(&e, "lw_msg").contains("HessianFcn"));
    assert_eq!(text(&e, "lw_id"), "optimoptions:fmincon:HessianFcn");

    // Solve emits no extra warning (no duplicate warning on consuming constructed options),
    // does not invoke HessianFcn, and solves the bounded quadratic via SQP.
    let solve_out = run_out(
        &mut e,
        "lastwarn('sentinel','sentinel:id'); \
         [x, fval, flag, out] = fmincon(@(x) 0.5*((x(1)-2)^2 + 4*(x(2)+1)^2), [0.5; -0.5], [], [], [], [], [0; -2], [1.5; 0], [], o); \
         alg = out.algorithm; msg = out.message; [after_msg, after_id] = lastwarn;",
    );
    assert!(
        solve_out.is_empty(),
        "unexpected solve output: {solve_out:?}"
    );
    assert_eq!(text(&e, "after_msg"), "sentinel");
    assert_eq!(text(&e, "after_id"), "sentinel:id");
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[1.5, -1.], 1e-6);
    close(&[scalar(&e, "fval")], &[0.125], 1e-6);
    assert_eq!(text(&e, "alg"), "sqp");
    assert!(text(&e, "msg").contains("mapped"));

    // 2. Analytic gradient + nonlinear equality problem agrees with existing SQP result,
    //    and HessianFcn handle is still not invoked.
    let grad_out = run_out(
        &mut e,
        "og = optimoptions(@fmincon,'Display','off','SpecifyObjectiveGradient',true,'SpecifyConstraintGradient',true,\
         'Algorithm','interior-point','HessianFcn',@(x,lambda) error('hessian_should_not_be_called'));",
    );
    assert_eq!(grad_out.len(), 2, "{grad_out:?}");
    let grad_solve = run_out(
        &mut e,
        "[xg, fg, flagg, outg, Lg] = fmincon(@objective_gradient, [0; 0], [], [], [], [], [], [], @eqcon, og); \
         msgg = outg.message; eqg = Lg.eqnonlin; fcg = outg.funcCount;",
    );
    assert!(grad_solve.is_empty(), "{grad_solve:?}");
    close(&values(&e, "xg"), &[0., 1.], 1e-6);
    assert_eq!(scalar(&e, "flagg"), 1.);
    assert!(text(&e, "msgg").contains("mapped"));
    close(&values(&e, "eqg"), &[4.], 1e-6);
    assert!(scalar(&e, "fcg") <= 20.);

    // 3. Single-value expression dispatch (optimoptions inside fmincon call) also warns once.
    let expr_out = run_out(
        &mut e,
        "lastwarn('',''); \
         x_expr = fmincon(@(x) (x-0.5)^2, 0, [], [], [], [], 0, 1, [], \
         optimoptions(\"fmincon\",'Display','off','Algorithm','active-set')); \
         [expr_msg, expr_id] = lastwarn;",
    );
    assert_eq!(expr_out.len(), 1, "{expr_out:?}");
    assert!(expr_out[0].contains("fmincon") && expr_out[0].contains("Algorithm"));
    assert_eq!(text(&e, "expr_id"), "optimoptions:fmincon:Algorithm");
    close(&values(&e, "x_expr"), &[0.5], 1e-6);
}

#[test]
fn fmincon_r2024b_options_validation_warnings_suppression_and_negatives() {
    let mut e = Engine::default();

    // 1. Supported no-warning settings (including Algorithm='sqp' and aliases).
    let quiet = run_out(
        &mut e,
        "lastwarn('sentinel','sentinel:id'); \
         oq = optimoptions('fmincon','Display','off','Algorithm','sqp',\
         'MaxIterations',100,'MaxFunctionEvaluations',500,'OptimalityTolerance',1e-8,\
         'StepTolerance',1e-10,'ConstraintTolerance',1e-8,'SpecifyObjectiveGradient',false,\
         'SpecifyConstraintGradient',false,'FiniteDifferenceType','forward',\
         'TolCon',1e-8,'TolX',1e-10,'GradObj',false,'GradConstr',false,'MaxIter',100,'MaxFunEvals',500,'FinDiffType','forward'); \
         [qm, qid] = lastwarn;",
    );
    assert!(quiet.is_empty(), "{quiet:?}");
    assert_eq!(text(&e, "qm"), "sentinel");
    assert_eq!(text(&e, "qid"), "sentinel:id");

    // 2. FunctionTolerance checked separately from OptimalityTolerance / TolFun:
    //    FunctionTolerance is accepted without error, but does not stop SQP early when OptimalityTolerance is tight;
    //    TolFun sets OptimalityTolerance and does stop SQP early when relaxed.
    //    The constructor warning for a nonempty FunctionTolerance is asserted in
    //    fmincon_warn_regression_function_tolerance.
    run(
        &mut e,
        "oft = optimoptions('fmincon','Display','off','FunctionTolerance',1e2,'OptimalityTolerance',1e-10); \
         [xft, ~, flagft, outft] = fmincon(@(x) (x-2)^2, 0, [], [], [], [], 0, 5, [], oft); \
         otf = optimoptions('fmincon','Display','off','TolFun',1e2); \
         [xtf, ~, flagtf, outtf] = fmincon(@(x) (x-2)^2, 0, [], [], [], [], 0, 5, [], otf);",
    );
    assert_eq!(scalar(&e, "flagft"), 1.);
    close(&values(&e, "xft"), &[2.], 1e-6);
    assert_eq!(scalar(&e, "flagtf"), 1.);
    // TolFun=1e2 relaxes OptimalityTolerance to 1e2 so SQP stops immediately at x0=0 (|grad|=4 <= 100).
    close(&values(&e, "xtf"), &[0.], 1e-12);

    // 3. Documented R2024b unimplemented fmincon options (bounded table coverage):
    //    case-insensitive, warn once with optimoptions:fmincon:<Canonical>, pass through to solve.
    for (given, canonical, value) in [
        ("algorithm", "Algorithm", "'interior-point'"),
        ("algorithm", "Algorithm", "'trust-region-reflective'"),
        ("algorithm", "Algorithm", "'sqp-legacy'"),
        ("algorithm", "Algorithm", "'active-set'"),
        ("checkgradients", "CheckGradients", "true"),
        ("diagnostics", "Diagnostics", "'on'"),
        ("diffmaxchange", "DiffMaxChange", "1e-1"),
        ("diffminchange", "DiffMinChange", "1e-8"),
        ("enablefeasibilitymode", "EnableFeasibilityMode", "true"),
        (
            "finitedifferencestepsize",
            "FiniteDifferenceStepSize",
            "[1e-6; 1e-6]",
        ),
        ("funvalcheck", "FunValCheck", "'on'"),
        ("hessianapproximation", "HessianApproximation", "'lbfgs'"),
        ("hessianfcn", "HessianFcn", "'objective'"),
        ("hessianmultiplyfcn", "HessianMultiplyFcn", "@(H,y) y"),
        ("honorbounds", "HonorBounds", "false"),
        ("initbarrierparam", "InitBarrierParam", "0.1"),
        ("inittrustregionradius", "InitTrustRegionRadius", "1.0"),
        ("maxprojcgiter", "MaxProjCGIter", "10"),
        ("maxsqpiter", "MaxSQPIter", "50"),
        ("objectivelimit", "ObjectiveLimit", "-1e10"),
        ("outputfcn", "OutputFcn", "@(x,ov,s) false"),
        ("plotfcn", "PlotFcn", "'optimplotfval'"),
        ("plotfcns", "PlotFcn", "@(x,ov,s) false"),
        ("rellinesrchbnd", "RelLineSrchBnd", "0.5"),
        ("rellinesrchbndduration", "RelLineSrchBndDuration", "2"),
        ("scaleproblem", "ScaleProblem", "'obj-and-constr'"),
        ("subproblemalgorithm", "SubproblemAlgorithm", "'cg'"),
        ("tolconsqp", "TolConSQP", "1e-6"),
        ("typicalx", "TypicalX", "[1; 1]"),
        ("useparallel", "UseParallel", "true"),
    ] {
        let out = run_out(
            &mut e,
            &format!(
                "lastwarn('',''); o = optimoptions('fmincon','Display','off','{given}',{value}); [m,id] = lastwarn;"
            ),
        );
        assert_eq!(out.len(), 1, "{given}={value}: {out:?}");
        assert!(
            out[0].contains("fmincon") && out[0].contains(canonical),
            "{given}={value}: {out:?}"
        );
        assert_eq!(text(&e, "id"), format!("optimoptions:fmincon:{canonical}"));
        assert!(text(&e, "m").contains(canonical));
        let solve_w = run_out(
            &mut e,
            "[x, f, flag] = fmincon(@(x) (x(1)-1)^2 + (x(2)+2)^2, [0; 0], [], [], [], [], [-2; -3], [2; 2], [], o);",
        );
        assert!(solve_w.is_empty(), "{given}={value}: {solve_w:?}");
        assert_eq!(scalar(&e, "flag"), 1., "{given}={value}");
        close(&values(&e, "x"), &[1., -2.], 1e-5);
    }

    // 4. Warning deduplication in one call and warning('off', id) suppression (lastwarn still updates).
    let dedup = run_out(
        &mut e,
        "o = optimoptions('fmincon','UseParallel',true,'UseParallel',false);",
    );
    assert_eq!(dedup.len(), 1, "{dedup:?}");
    let suppressed = run_out(
        &mut e,
        "warning('off','optimoptions:fmincon:CheckGradients'); lastwarn('',''); \
         o = optimoptions('fmincon','CheckGradients',true); [sm, sid] = lastwarn;",
    );
    assert!(suppressed.is_empty(), "{suppressed:?}");
    assert!(text(&e, "sm").contains("CheckGradients"));
    assert_eq!(text(&e, "sid"), "optimoptions:fmincon:CheckGradients");

    // 5. Invalid values, typos, paired-error no-warning, and other-solver negatives.
    for (call, expected) in [
        (
            "optimoptions('fmincon','Algorithm','dual-simplex')",
            "Algorithm",
        ),
        ("optimoptions('fmincon','Algorithm','bogus')", "Algorithm"),
        ("optimoptions('fmincon','HessianFcn',123)", "HessianFcn"),
        (
            "optimoptions('fmincon','HessianApproximation','sr1')",
            "HessianApproximation",
        ),
        (
            "optimoptions('fmincon','SubproblemAlgorithm','newton')",
            "SubproblemAlgorithm",
        ),
        (
            "optimoptions('fmincon','FiniteDifferenceStepSize',-1)",
            "FiniteDifferenceStepSize",
        ),
        (
            "optimoptions('fmincon','FiniteDifferenceStepSize',[1 2; 3 4])",
            "FiniteDifferenceStepSize",
        ),
        (
            "optimoptions('fmincon','CheckGradients',2)",
            "CheckGradients",
        ),
        ("optimoptions('fmincon','UseParallel',NaN)", "UseParallel"),
        ("optimoptions('fmincon','MaxSQPIter',0)", "MaxSQPIter"),
        (
            "optimoptions('fmincon','InitBarrierParam',-1)",
            "InitBarrierParam",
        ),
        ("optimoptions('fmincon','TolConSQP',0)", "TolConSQP"),
        (
            "optimoptions('fmincon','DiffMinChange',-1)",
            "DiffMinChange",
        ),
        ("optimoptions('fmincon','DiffMaxChange',0)", "DiffMaxChange"),
        (
            "optimoptions('fmincon','ObjectiveLimit',NaN)",
            "ObjectiveLimit",
        ),
        ("optimoptions('fmincon','TypicalX',Inf)", "TypicalX"),
        (
            "optimoptions('fmincon','ScaleProblem','bogus')",
            "ScaleProblem",
        ),
        (
            "optimoptions('fmincon','HessianFnc','objective')",
            "unknown option",
        ),
        ("optimoptions('fmincon','SwarmSize',10)", "unknown option"),
        ("optimoptions('fmincon','MaxNodes',10)", "not implemented"),
        // Other-solver negatives without changing those solvers:
        (
            "optimoptions('fsolve','HessianFcn','objective')",
            "unknown option",
        ),
        (
            "optimoptions('linprog','HessianFcn','objective')",
            "unknown option",
        ),
        (
            "optimoptions('quadprog','HessianFcn','objective')",
            "unknown option",
        ),
    ] {
        run(&mut e, "lastwarn('sentinel','sentinel:id');");
        let err = fails(&mut e, call);
        assert!(err.contains(expected), "{call}: {err}");
        run(&mut e, "[em, eid] = lastwarn;");
        assert_eq!(text(&e, "em"), "sentinel", "{call}");
        assert_eq!(text(&e, "eid"), "sentinel:id", "{call}");
    }

    // Valid option paired with invalid option emits no warning.
    run(&mut e, "lastwarn('sentinel','sentinel:id');");
    let pair_err = fails(
        &mut e,
        "optimoptions('fmincon','HessianFcn',@(x,l) x,'UseParallel',NaN);",
    );
    assert!(pair_err.contains("UseParallel"), "{pair_err}");
    run(&mut e, "[pm, pid] = lastwarn;");
    assert_eq!(text(&e, "pm"), "sentinel");

    // fminunc still works through optim_nl without interception.
    let fminunc_out = run_out(
        &mut e,
        "ou = optimoptions('fminunc','Display','off','Algorithm','trust-region'); \
         [xu, ~, flagu] = fminunc(@(x) (x-3)^2, 0, ou);",
    );
    assert_eq!(fminunc_out.len(), 1, "{fminunc_out:?}");
    assert!(fminunc_out[0].contains("fminunc"));
    assert_eq!(scalar(&e, "flagu"), 1.);
    close(&values(&e, "xu"), &[3.], 1e-5);
}

#[test]
fn fmincon_warn_regression_function_tolerance() {
    let mut e = Engine::default();

    // Nonempty explicit FunctionTolerance is validated and stored, but SQP stops
    // on optimality_tol. One suppressible warning at the named constructor.
    let ctor = run_out(
        &mut e,
        "lastwarn('',''); \
         o = optimoptions('fmincon','Display','off','FunctionTolerance',1e2,'OptimalityTolerance',1e-10); \
         [m,id] = lastwarn;",
    );
    assert_eq!(ctor.len(), 1, "{ctor:?}");
    assert!(ctor[0].contains("FunctionTolerance"), "{ctor:?}");
    assert!(ctor[0].contains("OptimalityTolerance"), "{ctor:?}");
    assert_eq!(text(&e, "id"), "optimoptions:fmincon:FunctionTolerance");
    assert!(text(&e, "m").contains("FunctionTolerance"));

    // Consuming the struct in a real solve must not warn again. The tight
    // OptimalityTolerance still drives the solve to x=2 (FunctionTolerance does not).
    let solve = run_out(
        &mut e,
        "lastwarn('sentinel','sentinel:id'); \
         [xft, ~, flagft] = fmincon(@(x) (x-2)^2, 0, [], [], [], [], 0, 5, [], o); \
         [sm, sid] = lastwarn;",
    );
    assert!(solve.is_empty(), "{solve:?}");
    assert_eq!(text(&e, "sm"), "sentinel");
    assert_eq!(text(&e, "sid"), "sentinel:id");
    assert_eq!(scalar(&e, "flagft"), 1.);
    close(&values(&e, "xft"), &[2.], 1e-6);

    // Effective OptimalityTolerance, legacy TolFun, the default, and an empty
    // reset are not requests to warn. TolFun after FunctionTolerance replaces it.
    for call in [
        "optimoptions('fmincon','Display','off','OptimalityTolerance',1e-8)",
        "optimoptions('fmincon','Display','off','TolFun',1e2)",
        "optimoptions('fmincon','Display','off','FunctionTolerance',[])",
        "optimoptions('fmincon','Display','off')",
        "optimoptions('fmincon','Display','off','FunctionTolerance',1e-2,'FunctionTolerance',[])",
        "optimoptions('fmincon','Display','off','FunctionTolerance',1e-2,'TolFun',1e-8)",
        "optimoptions('fminimax','Display','off','FunctionTolerance',1e-2)",
        "optimoptions(struct('Display','off'),'FunctionTolerance',1e-2)",
    ] {
        let out = run_out(
            &mut e,
            &format!("lastwarn('sentinel','sentinel:id'); oq = {call}; [qm,qid] = lastwarn;"),
        );
        assert!(out.is_empty(), "{call}: {out:?}");
        assert_eq!(text(&e, "qm"), "sentinel", "{call}");
        assert_eq!(text(&e, "qid"), "sentinel:id", "{call}");
    }

    // TolFun alone still relaxes OptimalityTolerance, so SQP stops at x0.
    let tolfun_solve = run_out(
        &mut e,
        "otf = optimoptions('fmincon','Display','off','TolFun',1e2); \
         [xtf, ~, flagtf] = fmincon(@(x) (x-2)^2, 0, [], [], [], [], 0, 5, [], otf);",
    );
    assert!(tolfun_solve.is_empty(), "{tolfun_solve:?}");
    assert_eq!(scalar(&e, "flagtf"), 1.);
    close(&values(&e, "xtf"), &[0.], 1e-12);

    // A FunctionTolerance that survives TolFun is the final request and warns once.
    let after_tolfun = run_out(
        &mut e,
        "lastwarn('',''); \
         o = optimoptions('fmincon','Display','off','TolFun',1e-8,'FunctionTolerance',1e-2); \
         [m,id] = lastwarn;",
    );
    assert_eq!(after_tolfun.len(), 1, "{after_tolfun:?}");
    assert_eq!(text(&e, "id"), "optimoptions:fmincon:FunctionTolerance");

    // @fmincon is a named constructor. An untagged update of an fmincon struct is not.
    let handle = run_out(
        &mut e,
        "lastwarn('',''); \
         oh = optimoptions(@fmincon,'Display','off','FunctionTolerance',1e-4); \
         [hm,hid] = lastwarn;",
    );
    assert_eq!(handle.len(), 1, "{handle:?}");
    assert_eq!(text(&e, "hid"), "optimoptions:fmincon:FunctionTolerance");
    run(
        &mut e,
        "base = optimoptions('fmincon','Display','off','ConstraintTolerance',1e-8); \
         lastwarn('sentinel','sentinel:id');",
    );
    let untagged = run_out(
        &mut e,
        "ou = optimoptions(base,'Algorithm','active-set','FunctionTolerance',1e-3); \
         [um,uid] = lastwarn;",
    );
    assert!(untagged.is_empty(), "{untagged:?}");
    assert_eq!(text(&e, "um"), "sentinel");
    assert_eq!(text(&e, "uid"), "sentinel:id");

    // Suppression silences the text; lastwarn still records the id. Invalid values error quietly.
    let suppressed = run_out(
        &mut e,
        "warning('off','optimoptions:fmincon:FunctionTolerance'); lastwarn('',''); \
         os = optimoptions('fmincon','FunctionTolerance',1e-3); [sm,sid] = lastwarn;",
    );
    assert!(suppressed.is_empty(), "{suppressed:?}");
    assert!(text(&e, "sm").contains("FunctionTolerance"));
    assert_eq!(text(&e, "sid"), "optimoptions:fmincon:FunctionTolerance");
    run(&mut e, "lastwarn('sentinel','sentinel:id');");
    let err = fails(&mut e, "optimoptions('fmincon','FunctionTolerance',0);");
    assert!(err.contains("FunctionTolerance"), "{err}");
    run(&mut e, "[em,eid] = lastwarn;");
    assert_eq!(text(&e, "em"), "sentinel");
    assert_eq!(text(&e, "eid"), "sentinel:id");
}

#[test]
fn fmincon_warn_regression_final_values() {
    let mut e = Engine::default();

    // active-set then sqp: the final request is the implemented algorithm.
    // Quiet, and the solve really uses SQP (no mapped note).
    let quiet = run_out(
        &mut e,
        "lastwarn('sentinel','sentinel:id'); \
         o = optimoptions('fmincon','Display','off','Algorithm','active-set','Algorithm','sqp'); \
         [m,id] = lastwarn; \
         [x,~,flag,out] = fmincon(@(x) (x-0.5)^2, 0, [], [], [], [], 0, 1, [], o); \
         alg = out.algorithm; msg = out.message; \
         [sm,sid] = lastwarn;",
    );
    assert!(quiet.is_empty(), "{quiet:?}");
    assert_eq!(text(&e, "m"), "sentinel");
    assert_eq!(text(&e, "id"), "sentinel:id");
    assert_eq!(text(&e, "sm"), "sentinel");
    assert_eq!(text(&e, "sid"), "sentinel:id");
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[0.5], 1e-6);
    assert_eq!(text(&e, "alg"), "sqp");
    assert!(!text(&e, "msg").contains("mapped"), "{}", text(&e, "msg"));

    // The reverse warns once and names active-set. Solve maps that final request to SQP.
    let rev = run_out(
        &mut e,
        "lastwarn('',''); \
         o = optimoptions('fmincon','Display','off','Algorithm','sqp','Algorithm','active-set'); \
         [m,id] = lastwarn;",
    );
    assert_eq!(rev.len(), 1, "{rev:?}");
    assert!(rev[0].contains("active-set"), "{rev:?}");
    assert!(!rev[0].contains("interior-point"), "{rev:?}");
    assert_eq!(text(&e, "id"), "optimoptions:fmincon:Algorithm");
    let rev_solve = run_out(
        &mut e,
        "lastwarn('sentinel','sentinel:id'); \
         [xr,~,flagr,outr] = fmincon(@(x) (x-0.5)^2, 0, [], [], [], [], 0, 1, [], o); \
         algr = outr.algorithm; msgr = outr.message; [sm,sid] = lastwarn;",
    );
    assert!(rev_solve.is_empty(), "{rev_solve:?}");
    assert_eq!(text(&e, "sm"), "sentinel");
    assert_eq!(text(&e, "sid"), "sentinel:id");
    assert_eq!(scalar(&e, "flagr"), 1.);
    close(&values(&e, "xr"), &[0.5], 1e-6);
    assert_eq!(text(&e, "algr"), "sqp");
    assert!(text(&e, "msgr").contains("mapped"), "{}", text(&e, "msgr"));
    assert!(
        text(&e, "msgr").contains("active-set"),
        "{}",
        text(&e, "msgr")
    );

    // Two unsupported requests: one warning, and it names the final algorithm.
    let two = run_out(
        &mut e,
        "lastwarn('',''); \
         o = optimoptions('fmincon','Display','off','Algorithm','interior-point','Algorithm','active-set'); \
         [m,id] = lastwarn;",
    );
    assert_eq!(two.len(), 1, "{two:?}");
    assert!(two[0].contains("active-set"), "{two:?}");
    assert!(!two[0].contains("interior-point"), "{two:?}");
    assert_eq!(text(&e, "id"), "optimoptions:fmincon:Algorithm");
    let replaced = run_out(
        &mut e,
        "lastwarn('',''); \
         o = optimoptions('fmincon','Display','off','Algorithm','interior-point','HessianFcn',@(x,l) x,'Algorithm','trust-region-reflective'); \
         [m,id] = lastwarn;",
    );
    assert_eq!(replaced.len(), 2, "{replaced:?}");
    assert!(
        replaced[0].contains("trust-region-reflective"),
        "{replaced:?}"
    );
    assert!(!replaced[0].contains("interior-point"), "{replaced:?}");
    assert!(replaced[1].contains("HessianFcn"), "{replaced:?}");
    assert_eq!(text(&e, "id"), "optimoptions:fmincon:HessianFcn");

    // Clearing an ineffective setting to empty drops the warning and the discarded value.
    let cleared_alg = run_out(
        &mut e,
        "lastwarn('sentinel','sentinel:id'); \
         o = optimoptions('fmincon','Display','off','Algorithm','active-set','Algorithm',[]); \
         [m,id] = lastwarn; \
         [xc,~,flagc,outc] = fmincon(@(x) (x-0.5)^2, 0, [], [], [], [], 0, 1, [], o); \
         msgc = outc.message; [sm,sid] = lastwarn;",
    );
    assert!(cleared_alg.is_empty(), "{cleared_alg:?}");
    assert_eq!(text(&e, "m"), "sentinel");
    assert_eq!(text(&e, "sm"), "sentinel");
    assert_eq!(text(&e, "sid"), "sentinel:id");
    assert_eq!(scalar(&e, "flagc"), 1.);
    close(&values(&e, "xc"), &[0.5], 1e-6);
    assert!(!text(&e, "msgc").contains("mapped"), "{}", text(&e, "msgc"));
    for call in [
        "optimoptions('fmincon','Display','off','HessianFcn',@(x,l) error('hessian_should_not_be_called'),'HessianFcn',[])",
        "optimoptions('fmincon','Display','off','UseParallel',true,'UseParallel',[])",
        "optimoptions('fmincon','Display','off','UseParallel',true,'UseParallel',[],'CheckGradients',[])",
        "optimoptions('quadprog','Algorithm','active-set','Display','off')",
        "optimoptions(struct('Algorithm','sqp'),'Algorithm','active-set')",
    ] {
        let out = run_out(
            &mut e,
            &format!("lastwarn('sentinel','sentinel:id'); oq = {call}; [qm,qid] = lastwarn;"),
        );
        assert!(out.is_empty(), "{call}: {out:?}");
        assert_eq!(text(&e, "qm"), "sentinel", "{call}");
        assert_eq!(text(&e, "qid"), "sentinel:id", "{call}");
    }

    // Overwriting Algorithm to sqp drops only that warning. HessianFcn still warns once.
    let kept = run_out(
        &mut e,
        "lastwarn('',''); \
         o = optimoptions('fmincon','Display','off','Algorithm','active-set','HessianFcn',@(x,l) x,'Algorithm','sqp'); \
         [m,id] = lastwarn;",
    );
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert!(kept[0].contains("HessianFcn"), "{kept:?}");
    assert!(!kept[0].contains("Algorithm"), "{kept:?}");
    assert_eq!(text(&e, "id"), "optimoptions:fmincon:HessianFcn");

    // A later invalid pair still fails the constructor and emits nothing.
    run(&mut e, "lastwarn('sentinel','sentinel:id');");
    let err = fails(
        &mut e,
        "optimoptions('fmincon','Algorithm','active-set','Algorithm','bogus');",
    );
    assert!(err.contains("Algorithm"), "{err}");
    run(&mut e, "[em,eid] = lastwarn;");
    assert_eq!(text(&e, "em"), "sentinel");
    assert_eq!(text(&e, "eid"), "sentinel:id");

    // Empty then a final unsupported request warns once for that final value.
    let restored = run_out(
        &mut e,
        "lastwarn('',''); \
         o = optimoptions('fmincon','Display','off','Algorithm','active-set','Algorithm',[],'Algorithm','interior-point'); \
         [m,id] = lastwarn;",
    );
    assert_eq!(restored.len(), 1, "{restored:?}");
    assert!(restored[0].contains("interior-point"), "{restored:?}");
    assert!(!restored[0].contains("active-set"), "{restored:?}");
    assert_eq!(text(&e, "id"), "optimoptions:fmincon:Algorithm");
}

fn assert_tiny_lp_solve(e: &mut Engine, mapped: Option<&str>) {
    let out = run_out(
        e,
        "lastwarn('sentinel','sentinel:id'); \
         [x,f,flag,out] = linprog([-1;-1],[1 1],2,[],[],[0;0],[2;0],o); \
         alg = out.algorithm; msg = out.message; [sm,sid] = lastwarn;",
    );
    assert!(out.is_empty(), "{out:?}");
    assert_eq!(text(e, "sm"), "sentinel");
    assert_eq!(text(e, "sid"), "sentinel:id");
    assert_eq!(scalar(e, "flag"), 1.);
    close(&values(e, "x"), &[2., 0.], 1e-5);
    close(&values(e, "f"), &[-2.], 1e-5);
    assert_eq!(text(e, "alg"), "primal-dual interior-point");
    let msg = text(e, "msg");
    match mapped {
        Some(name) => {
            assert!(msg.contains("mapped"), "{msg}");
            assert!(msg.contains(name), "{msg}");
        }
        None => assert!(!msg.contains("mapped"), "{msg}"),
    }
}

fn assert_tiny_qp_solve(e: &mut Engine, mapped: Option<&str>) {
    let out = run_out(
        e,
        "lastwarn('sentinel','sentinel:id'); \
         [x,f,flag,out] = quadprog(2,-2,[],[],[],[],0,2,[],o); \
         alg = out.algorithm; msg = out.message; [sm,sid] = lastwarn;",
    );
    assert!(out.is_empty(), "{out:?}");
    assert_eq!(text(e, "sm"), "sentinel");
    assert_eq!(text(e, "sid"), "sentinel:id");
    assert_eq!(scalar(e, "flag"), 1.);
    close(&values(e, "x"), &[1.], 1e-5);
    close(&values(e, "f"), &[-1.], 1e-5);
    assert_eq!(text(e, "alg"), "primal-dual interior-point");
    let msg = text(e, "msg");
    match mapped {
        Some(name) => {
            assert!(msg.contains("mapped"), "{msg}");
            assert!(msg.contains(name), "{msg}");
        }
        None => assert!(!msg.contains("mapped"), "{msg}"),
    }
}

#[test]
fn linprog_r2024b_option_contract() {
    // R2024b linprog options: Algorithm, ConstraintTolerance, Diagnostics, Display,
    // MaxIterations, MaxTime, OptimalityTolerance, Preprocess, Presolve.
    let mut e = Engine::default();

    // 1. Supported options and preserved compatibility aliases emit no warnings.
    for alg in ["interior-point", "interior-point-legacy", "dual-simplex"] {
        let out = run_out(
            &mut e,
            &format!(
                "lastwarn('sentinel','sentinel:id'); \
                 o = optimoptions('linprog','Display','off','Algorithm','{alg}',\
                 'MaxIterations',80,'OptimalityTolerance',1e-9,'ConstraintTolerance',1e-8,\
                 'MaxIter',80,'TolCon',1e-8,'TolFun',1e-9); \
                 [qm,qid] = lastwarn;"
            ),
        );
        assert!(out.is_empty(), "{alg}: {out:?}");
        assert_eq!(text(&e, "qm"), "sentinel", "{alg}");
        assert_eq!(text(&e, "qid"), "sentinel:id", "{alg}");
        assert_tiny_lp_solve(&mut e, (alg != "interior-point").then_some(alg));
    }

    // 2. Documented unimplemented algorithms warn once at construction, solve cleanly, respect suppression.
    for alg in ["dual-simplex-highs", "dual-simplex-legacy"] {
        let out = run_out(
            &mut e,
            &format!(
                "lastwarn('',''); \
                 o = optimoptions('linprog','Display','off','Algorithm','{alg}'); \
                 [wm,wid] = lastwarn;"
            ),
        );
        assert_eq!(out.len(), 1, "{alg}: {out:?}");
        assert!(
            out[0].contains("linprog") && out[0].contains(alg),
            "{out:?}"
        );
        assert_eq!(text(&e, "wid"), "optimoptions:linprog:Algorithm");
        assert!(text(&e, "wm").contains(alg));
        assert_tiny_lp_solve(&mut e, Some(alg));
    }
    let suppressed = run_out(
        &mut e,
        "warning('off','optimoptions:linprog:Algorithm'); lastwarn('',''); \
         o = optimoptions('linprog','Display','off','Algorithm','dual-simplex-highs'); \
         [sm,sid] = lastwarn;",
    );
    assert!(suppressed.is_empty(), "{suppressed:?}");
    assert_eq!(text(&e, "sid"), "optimoptions:linprog:Algorithm");
    assert!(text(&e, "sm").contains("dual-simplex-highs"));
    assert_tiny_lp_solve(&mut e, Some("dual-simplex-highs"));
    run(&mut e, "warning('on','optimoptions:linprog:Algorithm');");

    // 3. Documented ineffective options warn once at construction and pass through real solves.
    for (given, canonical, value) in [
        ("diagnostics", "Diagnostics", "'on'"),
        ("MaxTime", "MaxTime", "10"),
        ("presolve", "Presolve", "'off'"),
        ("Preprocess", "Preprocess", "'none'"),
    ] {
        let out = run_out(
            &mut e,
            &format!(
                "lastwarn('',''); \
                 o = optimoptions('linprog','Display','off','{given}',{value}); \
                 [wm,wid] = lastwarn;"
            ),
        );
        assert_eq!(out.len(), 1, "{given}: {out:?}");
        assert!(
            out[0].contains("linprog") && out[0].contains(canonical),
            "{out:?}"
        );
        assert_eq!(text(&e, "wid"), format!("optimoptions:linprog:{canonical}"));
        assert_tiny_lp_solve(&mut e, None);
    }

    // 4. Empty reset in the same call drops the warning; unknown/typo/wrong-value/exclusive options error quietly.
    let cleared = run_out(
        &mut e,
        "lastwarn('sentinel','sentinel:id'); \
         o = optimoptions('linprog','Display','off','MaxTime',5,'MaxTime',[],'Algorithm','dual-simplex-highs','Algorithm','interior-point'); \
         [cm,cid] = lastwarn;",
    );
    assert!(cleared.is_empty(), "{cleared:?}");
    assert_eq!(text(&e, "cm"), "sentinel");
    assert_eq!(text(&e, "cid"), "sentinel:id");

    for (call, part) in [
        ("optimoptions('linprog','Algorithm','sqp')", "Algorithm"),
        (
            "optimoptions('linprog','MaxIterationsx',10)",
            "MaxIterationsx",
        ),
        ("optimoptions('linprog','MaxTime',-1)", "MaxTime"),
        ("optimoptions('linprog','Presolve','bogus')", "Presolve"),
        ("optimoptions('linprog','Preprocess','bogus')", "Preprocess"),
        (
            "optimoptions('linprog','FunctionTolerance',1e-6)",
            "FunctionTolerance",
        ),
        (
            "optimoptions('linprog','StepTolerance',1e-6)",
            "StepTolerance",
        ),
        (
            "optimoptions('linprog','MaxFunctionEvaluations',100)",
            "MaxFunctionEvaluations",
        ),
        (
            "optimoptions('linprog','LinearSolver','dense')",
            "LinearSolver",
        ),
        ("optimoptions('linprog','MaxNodes',5)", "MaxNodes"),
        (
            "optimoptions('linprog','SpecifyConstraintGradient',true)",
            "SpecifyConstraintGradient",
        ),
        (
            "linprog([-1;-1],[1 1],2,[],[],[0;0],[2;0],struct('FunctionTolerance',1e-6))",
            "FunctionTolerance",
        ),
        (
            "linprog([-1;-1],[1 1],2,[],[],[0;0],[2;0],struct('LinearSolver','dense'))",
            "LinearSolver",
        ),
        (
            "linprog([-1;-1],[1 1],2,[],[],[0;0],[2;0],struct('MaxIterationsx',10))",
            "MaxIterationsx",
        ),
    ] {
        run(&mut e, "lastwarn('sentinel','sentinel:id');");
        let err = fails(&mut e, call);
        assert!(err.contains(part), "{call}: {err}");
        run(&mut e, "[em,eid] = lastwarn;");
        assert_eq!(text(&e, "em"), "sentinel", "{call}");
        assert_eq!(text(&e, "eid"), "sentinel:id", "{call}");
    }
}

#[test]
fn quadprog_r2024b_option_contract() {
    // R2024b quadprog options + preserved compatibility algorithm aliases.
    let mut e = Engine::default();

    // 1. Documented unimplemented algorithms warn once at construction, solve cleanly, respect suppression.
    for alg in ["trust-region-reflective", "interior-point-convex"] {
        let out = run_out(
            &mut e,
            &format!(
                "lastwarn('',''); \
                 o = optimoptions('quadprog','Display','off','Algorithm','{alg}'); \
                 [wm,wid] = lastwarn;"
            ),
        );
        assert_eq!(out.len(), 1, "{alg}: {out:?}");
        assert!(
            out[0].contains("quadprog") && out[0].contains(alg),
            "{out:?}"
        );
        assert_eq!(text(&e, "wid"), "optimoptions:quadprog:Algorithm");
        assert!(text(&e, "wm").contains(alg));
        assert_tiny_qp_solve(&mut e, Some(alg));
    }
    let suppressed = run_out(
        &mut e,
        "warning('off','optimoptions:quadprog:Algorithm'); lastwarn('',''); \
         o = optimoptions('quadprog','Display','off','Algorithm','trust-region-reflective'); \
         [sm,sid] = lastwarn;",
    );
    assert!(suppressed.is_empty(), "{suppressed:?}");
    assert_eq!(text(&e, "sid"), "optimoptions:quadprog:Algorithm");
    assert_tiny_qp_solve(&mut e, Some("trust-region-reflective"));
    run(&mut e, "warning('on','optimoptions:quadprog:Algorithm');");

    // 2. Documented ineffective options warn once at construction and pass through real solves.
    for (given, canonical, value) in [
        ("diagnostics", "Diagnostics", "'on'"),
        ("FunctionTolerance", "FunctionTolerance", "1e-4"),
        ("HessMult", "HessianMultiplyFcn", "@(H,v) H*v"),
        ("LinearSolver", "LinearSolver", "'dense'"),
        ("MaxPCGIter", "MaxPCGIter", "10"),
        ("ObjectiveLimit", "ObjectiveLimit", "-1e12"),
        ("PrecondBandWidth", "PrecondBandWidth", "0"),
        ("TolX", "StepTolerance", "1e-8"),
        ("SubproblemAlgorithm", "SubproblemAlgorithm", "'cg'"),
        ("TolPCG", "TolPCG", "1e-2"),
        ("TypicalX", "TypicalX", "[1;1]"),
    ] {
        let out = run_out(
            &mut e,
            &format!(
                "lastwarn('',''); \
                 o = optimoptions('quadprog','Display','off','{given}',{value}); \
                 [wm,wid] = lastwarn;"
            ),
        );
        assert_eq!(out.len(), 1, "{given}: {out:?}");
        assert!(
            out[0].contains("quadprog") && out[0].contains(canonical),
            "{out:?}"
        );
        assert_eq!(
            text(&e, "wid"),
            format!("optimoptions:quadprog:{canonical}")
        );
        assert_tiny_qp_solve(&mut e, None);
    }

    // 3. Unknown/typo/wrong-value/exclusive options error quietly.
    for (call, part) in [
        (
            "optimoptions('quadprog','Algorithm','dual-simplex-highs')",
            "Algorithm",
        ),
        (
            "optimoptions('quadprog','MaxIterationsx',10)",
            "MaxIterationsx",
        ),
        (
            "optimoptions('quadprog','LinearSolver','bogus')",
            "LinearSolver",
        ),
        ("optimoptions('quadprog','TolPCG',0)", "TolPCG"),
        ("optimoptions('quadprog','MaxTime',5)", "MaxTime"),
        ("optimoptions('quadprog','Presolve','off')", "Presolve"),
        (
            "optimoptions('quadprog','MaxFunctionEvaluations',100)",
            "MaxFunctionEvaluations",
        ),
        (
            "optimoptions('quadprog','SpecifyObjectiveGradient',true)",
            "SpecifyObjectiveGradient",
        ),
        ("optimoptions('quadprog','MaxNodes',5)", "MaxNodes"),
        (
            "optimoptions('quadprog','SpecifyConstraintGradient',true)",
            "SpecifyConstraintGradient",
        ),
        (
            "quadprog(2,-2,[],[],[],[],0,2,[],struct('MaxTime',5))",
            "MaxTime",
        ),
        (
            "quadprog(2,-2,[],[],[],[],0,2,[],struct('MaxIterationsx',10))",
            "MaxIterationsx",
        ),
    ] {
        run(&mut e, "lastwarn('sentinel','sentinel:id');");
        let err = fails(&mut e, call);
        assert!(err.contains(part), "{call}: {err}");
        run(&mut e, "[em,eid] = lastwarn;");
        assert_eq!(text(&e, "em"), "sentinel", "{call}");
        assert_eq!(text(&e, "eid"), "sentinel:id", "{call}");
    }
}

#[test]
fn lp_qp_returned_options_tolfun_and_empty_alias_reset() {
    let mut e = Engine::default();

    // 1. Returned linprog options updated with TolFun are accepted and consumed without warning.
    for base in [
        "optimoptions('linprog','Display','off')",
        "optimoptions('linprog','Display','off','ConstraintTolerance',1e-7)",
    ] {
        let out = run_out(
            &mut e,
            &format!(
                "lastwarn('sentinel','sentinel:id'); \
                 o = {base}; o = optimoptions(o,'TolFun',1e-8); \
                 tol = o.OptimalityTolerance; [wm,wid] = lastwarn;"
            ),
        );
        assert!(out.is_empty(), "{base}: {out:?}");
        assert_eq!(scalar(&e, "tol"), 1e-8);
        assert_eq!(text(&e, "wm"), "sentinel");
        assert_eq!(text(&e, "wid"), "sentinel:id");
        assert_tiny_lp_solve(&mut e, None);
    }

    // 2. Empty canonical option resets without resurrecting its legacy alias (both optimoptions and struct).
    run(
        &mut e,
        "o = optimoptions('linprog','Display','off'); \
         p = optimoptions(o,'TolFun',1e-8); \
         u = optimoptions(p,'OptimalityTolerance',[]); \
         [x,f,flag,out] = linprog([-1;-1],[1 1],2,[],[],[0;0],[2;0],u); \
         alg = out.algorithm;",
    );
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[2., 0.], 1e-5);
    close(&values(&e, "f"), &[-2.], 1e-5);
    assert_eq!(text(&e, "alg"), "primal-dual interior-point");

    for (update, expected) in [
        (
            "u = struct('MaxIter',3,'TolFun',.01,'OptimalityTolerance',[]);",
            [0., 3.],
        ),
        ("u = struct('MaxIter',3,'MaxIterations',[]);", [1., 3.]),
    ] {
        run(
            &mut e,
            &format!(
                "{update} [x,f,k,r] = quadprog(2,-2,[],[],[],[],-3,3,0,u); kr = [k r.iterations];"
            ),
        );
        let actual = values(&e, "kr");
        if update.contains("MaxIterations") {
            // MaxIterations=[] resets to default (400), so quadprog converges to [1, >3].
            assert_eq!(actual[0], 1., "{update}: {actual:?}");
            assert!(actual[1] > 3., "{update}: {actual:?}");
        } else {
            assert_eq!(actual, expected, "{update}");
        }
    }
}
