//! Explicit cvxpy/OSQP state-trajectory reference, independently assembled in
//! tool/refs/mpc/mpcmove.py. Cost includes constant terms and has no factor 1/2.
use matrust::{Array, Engine, Matrix};
use serde_json::Value;

fn mat(e: &Engine, name: &str) -> Matrix {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m,
        a => panic!("{name}: {a:?}"),
    }
}
fn compare(m: &Matrix, want: &Value, label: &str, tolerance: f64) -> f64 {
    let rows = want.as_array().unwrap();
    assert_eq!(m.rows(), rows.len(), "{label}");
    let mut error: f64 = 0.;
    for (i, row) in rows.iter().enumerate() {
        let row = row.as_array().unwrap();
        assert_eq!(m.cols(), row.len(), "{label}");
        for (j, v) in row.iter().enumerate() {
            error = error.max((m.get(i, j) - v.as_f64().unwrap()).abs());
        }
    }
    assert!(
        error < tolerance,
        "{label}: maximum absolute error {error:e}"
    );
    error
}
fn scalar(e: &Engine, name: &str) -> f64 {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m.data()[0],
        Array::Logical(m) => f64::from(m.data()[0]),
        a => panic!("{name}: {a:?}"),
    }
}

#[test]
fn uncondensed_osqp_moves_trajectories_cost_and_closed_form_feedback() {
    let data: Value = serde_json::from_str(include_str!("fixtures/mpc/mpcmove.json")).unwrap();
    for case in data["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut e = Engine::default();
        e.execute(case["source"].as_str().unwrap())
            .unwrap_or_else(|err| panic!("{name} setup: {err}"));
        e.execute("alias=s;").unwrap();
        let mut max_move: f64 = 0.;
        let mut max_closed: f64 = 0.;
        for (step, v) in case["steps"].as_array().unwrap().iter().enumerate() {
            e.execute(v["call"].as_str().unwrap())
                .unwrap_or_else(|err| panic!("{name} step {step}: {err}"));
            e.execute("Uopt=info.Uopt;Yopt=info.Yopt;Xopt=info.Xopt;Topt=info.Topt;Slack=info.Slack;Cost=info.Cost;Plant=s.Plant;Disturbance=s.Disturbance;Noise=s.Noise;LastMove=s.LastMove;aliasPlant=alias.Plant;ok=strcmp(info.QPCode,'feasible');iterations=info.Iterations;").unwrap();
            if v["expected"]["QPCode"] == "infeasible" {
                e.execute("bad=strcmp(info.QPCode,'infeasible');").unwrap();
                assert_eq!(scalar(&e, "bad"), 1.);
                assert_eq!(scalar(&e, "iterations"), -1.);
                compare(&mat(&e, "mv"), &v["expected"]["mv"], name, 1e-12);
                eprintln!("{name}: max move error=0, QPCode=infeasible");
                continue;
            }
            assert_eq!(scalar(&e, "ok"), 1., "{name}");
            assert!(scalar(&e, "iterations") > 0., "{name}");
            let want = &v["expected"];
            max_move = max_move.max(compare(&mat(&e, "mv"), &want["mv"], name, 1e-6));
            for key in [
                "Uopt",
                "Yopt",
                "Xopt",
                "Topt",
                "Plant",
                "Disturbance",
                "Noise",
            ] {
                compare(
                    &mat(&e, key),
                    &want[key],
                    &format!("{name} {step} {key}"),
                    1e-6,
                );
            }
            for key in ["Cost", "Slack"] {
                let error = (scalar(&e, key) - want[key].as_f64().unwrap()).abs();
                assert!(error < 1e-6, "{name} {step} {key}: {error:e}");
            }
            if !want["closed"].is_null() {
                max_closed = max_closed.max(compare(
                    &mat(&e, "mv"),
                    &want["closed"],
                    &format!("{name} closed form"),
                    1e-9,
                ));
            }
            assert_eq!(mat(&e, "LastMove").data(), mat(&e, "mv").data());
            assert_eq!(mat(&e, "Plant").data(), mat(&e, "aliasPlant").data());
            let u = mat(&e, "Uopt");
            for j in 0..u.cols() {
                assert_eq!(u.get(u.rows() - 1, j), u.get(u.rows() - 2, j));
            }
        }
        eprintln!("{name}: max move error={max_move:e}, closed-form error={max_closed:e}");
        if name == "mimo_soft_ov" || name == "soft_mv_and_rate" {
            assert!(scalar(&e, "Slack") > 1e-3);
        }
    }
}

#[test]
fn infeasible_hard_constraints_and_iteration_limit_hold_last_move() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(0.8,0.2,1,0,0.1));c.MV.Min=1;c.MV.RateMax=0;s=mpcstate(c);s.LastMove=0.2;alias=s;[mv,info]=mpcmove(c,s,0,1);flag=info.Iterations;ok=strcmp(info.QPCode,'infeasible');last=alias.LastMove;").unwrap();
    assert_eq!(scalar(&e, "mv"), 0.2);
    assert_eq!(scalar(&e, "last"), 0.2);
    assert_eq!(scalar(&e, "flag"), -1.);
    assert_eq!(scalar(&e, "ok"), 1.);
    e.execute("c=mpc(ss(0.8,0.2,1,0,0.1));c.Optimizer.MaxIterations=1;c.MV.Max=0.1;c.MV.RateMax=0.05;s=mpcstate(c);[mv,info]=mpcmove(c,s,0,1);flag=info.Iterations;ok=strcmp(info.QPCode,'unreliable');").unwrap();
    assert_eq!(scalar(&e, "flag"), 0.);
    assert_eq!(scalar(&e, "mv"), 0.);
    assert_eq!(scalar(&e, "ok"), 1.);
    // Positive weights that underflow when squared still exercise a genuine
    // numerical Hessian failure, distinct from mathematical infeasibility.
    e.execute("c=mpc(ss(0.8,[0.2 0],1,[0 0],0.1));c.Weights.MVRate=[0.1 1e-200];s=mpcstate(c);s.LastMove=[.2;.3];[mv,info]=mpcmove(c,s,0,1);flag=info.Iterations;ok=strcmp(info.QPCode,'unreliable');last=s.LastMove;").unwrap();
    assert_eq!(scalar(&e, "flag"), -2.);
    assert_eq!(scalar(&e, "ok"), 1.);
    assert_eq!(mat(&e, "mv").data(), &[0.2, 0.3]);
    assert_eq!(mat(&e, "last").data(), &[0.2, 0.3]);
    // An actual-MV override does not replace LastMove when the QP fails.
    e.execute("c=mpc(ss(0.8,0.2,1,0,0.1));c.MV.Min=1;c.MV.RateMax=0;s=mpcstate(c);s.LastMove=0.2;[mv,info]=mpcmove(c,s,0,1,struct('MVUsed',0.3));").unwrap();
    assert_eq!(scalar(&e, "mv"), 0.2);
}

#[test]
fn runtime_bound_profile_overrides_are_local_and_extend_last_row() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(0.8,0.2,1,0,0.1),[],4,2);c.MV.Max=[Inf;0.4;0.5;0.6];c.MV.Min=-1;c.OV.Max=2;c.OV.Min=-2;o=mpcmoveopt;o.MVMax=0.2;o.MVMin=-0.5;o.OutputMax=[1;0.8];o.OutputMin=-1;s=mpcstate(c);[mv,info]=mpcmove(c,s,0,2,[],o);u=info.Uopt;original=c.MV.Max;d=c;d.MV.Max=[Inf;0.2;0.3;0.4];d.MV.Min=-0.5;d.OV.Max=[1;0.8];d.OV.Min=-1;t=mpcstate(d);[mv2,info2]=mpcmove(d,t,0,2);u2=info2.Uopt;").unwrap();
    let a = mat(&e, "u");
    let b = mat(&e, "u2");
    for (x, y) in a.data().iter().zip(b.data()) {
        assert!((x - y).abs() < 1e-10);
    }
    assert_eq!(mat(&e, "original").data(), &[f64::INFINITY, 0.4, 0.5, 0.6]);
    // A matrix override discards a pre-existing time-varying profile.
    e.execute("o.MVMax=[0.15;0.25];s=mpcstate(c);[mv,info]=mpcmove(c,s,0,2,o);u=info.Uopt;")
        .unwrap();
    assert!(scalar(&e, "mv") <= 0.15 + 1e-8);
}

#[test]
fn output_bound_overrides_match_properties_with_reordered_measurements() {
    for (bound, reference, limits) in [("Max", 1, ".05 .8"), ("Min", -1, "-.05 -.8")] {
        let mut e = Engine::default();
        e.execute(&format!(
            "p=ss([.8 0;.1 .7],[.2 0;.1 .3],eye(2),zeros(2),.1);\
             p=setmpcsignals(p,'MO',2,'UO',1);c=mpc(p);\
             c.OV(1).{bound}={reference}*2;c.OV(2).{bound}={reference}*2;\
             c.OV(1).{bound}ECR=0;c.OV(2).{bound}ECR=0;\
             d=c;b=[{limits}];d.OV(1).{bound}=b(1);d.OV(2).{bound}=b(2);\
             s=mpcstate(c);t=mpcstate(d);o=mpcmoveopt;o.Output{bound}=b;\
             [u,iu]=mpcmove(c,s,0,[{reference} {reference}],[],o);\
             [v,iv]=mpcmove(d,t,0,[{reference} {reference}]);\
             assert(strcmp(iu.QPCode,'feasible'));assert(strcmp(iv.QPCode,'feasible'));\
             assert(max(abs(u-v))<1e-9);assert(max(max(abs(iu.Yopt-iv.Yopt)))<1e-9);"
        ))
        .unwrap();
    }
}

#[test]
fn nominal_mv_target_defaults_custom_estimation_and_optional_inputs() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(0.8,0.2,1,0,0.1));c.Model.Nominal=struct('X',2,'U',3,'Y',5,'DX',0);c.Weights.MV=2;s=mpcstate(c);[mv,info]=mpcmove(c,s,[],[]);target=strcmp(c.MV.Target,'nominal');").unwrap();
    assert_eq!(scalar(&e, "target"), 1.);
    assert!((scalar(&e, "mv") - 3.).abs() < 1e-10);
    e.execute("c.MV.Target=4;s=mpcstate(c);mv=mpcmove(c,s,[],[]);")
        .unwrap();
    assert!(scalar(&e, "mv") > 3.1);
    e.execute("setEstimator(c,'custom');s=mpcstate(c);s.Plant=2.4;s.Disturbance=0.2;before=s.Plant;beforeD=s.Disturbance;[mv,info]=mpcmove(c,s);after=s.Plant;afterD=s.Disturbance;last=s.LastMove;").unwrap();
    assert_eq!(mat(&e, "before").data(), mat(&e, "after").data());
    assert_eq!(mat(&e, "beforeD").data(), mat(&e, "afterD").data());
    assert_eq!(scalar(&e, "last"), scalar(&e, "mv"));
    e.execute("c.MV.Target=[3;3.1;3.2];s=mpcstate(c);[mv,info]=mpcmove(c,s,[],5);")
        .unwrap();
}

#[test]
fn options_validation_current_horizon_limits_and_cancellation() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(0.8,0.2,1,0,0.1));s=mpcstate(c);")
        .unwrap();
    for field in [
        "OnlyComputeCost",
        "CustomConstraint",
        "MVTarget",
        "PredictionHorizon",
        "ControlHorizon",
        "uifigure",
    ] {
        let err = e
            .execute(&format!("mpcmove(c,s,0,1,struct('{field}',[]));"))
            .unwrap_err();
        assert!(
            err.contains("not supported") && err.contains(field),
            "{err}"
        );
    }
    for bad in [
        "mpcmoveopt(1)",
        "mpcmove(c,1,0,1)",
        "mpcmove(c,s,[1;2],1)",
        "mpcmove(c,s,NaN,1)",
        "mpcmove(c,s,0,ones(11,1))",
        "mpcmove(c,s,0,1,1)",
        "mpcmove(c,s,0,1,struct('MVWeights',-1))",
        "mpcmove(c,s,0,1,struct('MVWeights',ones(2,1)))",
        "mpcmove(c,s,0,1,struct('MVMax',0.2))",
        "mpcmove(c,s,0,1,struct('ECRWeight',0))",
        "mpcmove(c,s,0,1,struct('MVUsed',[0 1]))",
        "mpcmove(c,s,0,1,struct('OutputWeights',1i))",
        "mpcmove(c,s,0,1,struct('MVMax',NaN))",
    ] {
        assert!(e.execute(bad).is_err(), "{bad}");
    }
    // Lowering p is safe even if a previously valid bound/target no longer fits.
    e.execute("c.MV.Max=ones(10,1);c.PredictionHorizon=4;")
        .unwrap();
    let err = e.execute("mpcmove(c,s,0,1)").unwrap_err();
    assert!(err.contains("current PredictionHorizon"), "{err}");
    e.execute("c.MV.Max=1;c.MV.Target=ones(4,1);c.PredictionHorizon=3;")
        .unwrap();
    assert!(
        e.execute("mpcmove(c,s,0,1)")
            .unwrap_err()
            .contains("MV.Target")
    );
    e.execute("c.MV.Target='nominal';c.Weights.OV=ones(3,1);")
        .unwrap();
    assert!(e.execute("c.PredictionHorizon=2;").is_err());
    e.execute("o=mpcmoveopt;o.OutputWeights=[1;2];[mv,info]=mpcmove(c,s,0,1,o);")
        .unwrap();
    e.execute("c=mpc(ss(0.8,0.2,1,0,0.1),[],201,200);s=mpcstate(c);")
        .unwrap();
    assert!(
        e.execute("mpcmove(c,s,0,1)")
            .unwrap_err()
            .contains("200 variables")
    );
    let mut limits = e.limits_active();
    limits.memory_bytes = 50_000;
    e.limits_set(limits);
    e.execute("clear c s;").unwrap();
    // Set a tight budget after construction, matching the QP's host-limit tests.
    let mut e = Engine::default();
    e.execute("c=mpc(ss(0.8,0.2,1,0,0.1));s=mpcstate(c);")
        .unwrap();
    let mut limits = e.limits_active();
    limits.memory_bytes = 50_000;
    e.limits_set(limits);
    assert!(
        e.execute("mpcmove(c,s,0,1)")
            .unwrap_err()
            .contains("memory")
    );
    let mut e = Engine::default();
    e.execute("c=mpc(ss(0.8,0.2,1,0,0.1));s=mpcstate(c);")
        .unwrap();
    e.cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(e.execute("mv=mpcmove(c,s,0,1)").is_err());
    assert!(e.array("mv").is_none());
}

#[test]
#[ignore = "release benchmark; run with RAYON_NUM_THREADS=1 and 4"]
fn benchmark_condensed_move() {
    for (p, m, repeats) in [(10, 2, 100), (40, 12, 30)] {
        let mut e = Engine::default();
        e.execute(&format!("c=mpc(ss([0.8 0.1;0 0.7],[0.2 0.05;0.1 0.3],eye(2),zeros(2),0.1),[],{p},{m});c.MV(1).Max=0.6;c.MV(2).Max=0.6;s=mpcstate(c);")).unwrap();
        e.execute("[mv,info]=mpcmove(c,s,[0;0],[1 1]);").unwrap();
        let start = std::time::Instant::now();
        for _ in 0..repeats {
            e.execute("[mv,info]=mpcmove(c,s,[0;0],[1 1]);").unwrap();
        }
        eprintln!(
            "{repeats} MPC moves with Info: p={p}, m={m}, 2 MVs; {:?}; RAYON_NUM_THREADS={:?}; limits 200 variables/4000 inequalities/128 augmented states",
            start.elapsed(),
            std::env::var("RAYON_NUM_THREADS")
        );
    }
}

#[test]
fn mpc_consolidated_physical_output_bound_weight_and_nominal() {
    let mut e = Engine::default();
    e.execute("p=setmpcsignals(ss(.8,.2,[2;1],[0;0],.1),'MO',2,'UO',1); c=mpc(p,.1,1,1); c.OV(1).Max=.1; c.OV(1).MaxECR=0; s=mpcstate(c); [u,i]=mpcmove(c,s,0,[2 1]); y=i.Yopt;").unwrap();
    let bound = scalar(&e, "u");
    let y = mat(&e, "y");
    e.execute("c.OV(2).Max=100; s=mpcstate(c); [uo,io]=mpcmove(c,s,0,[2 1],struct('OutputMax',[.1 100]));").unwrap();
    let override_move = scalar(&e, "uo");
    e.execute("p=setmpcsignals(ss(.8,.2,[1;2],[0;0],.1),'MO',2,'UO',1); c=mpc(p,.1,1,1); c.Weights.OV=[1 0]; s=mpcstate(c); [u,i]=mpcmove(c,s,0,[1 0]);").unwrap();
    let weighted = scalar(&e, "u");
    e.execute("c.Model.Nominal.X=2; c.Model.Nominal.U=3; c.Model.Nominal.Y=[10;20]; c.OV(1).ScaleFactor=2; c.OV(2).ScaleFactor=5; s=mpcstate(c); [u,i]=mpcmove(c,s,20,[11 20]);").unwrap();
    let nominal = scalar(&e, "u");
    eprintln!(
        "physical anchors: bound={bound}, override={override_move}, weighted={weighted}, nominal={nominal}"
    );
    assert!((bound - 0.25).abs() < 1e-6, "physical bound move {bound}");
    assert!(y.get(1, 0) <= 0.1 + 1e-7);
    assert!((override_move - 0.25).abs() < 1e-6);
    assert!((weighted - 4.).abs() < 1e-6);
    assert!((nominal - 5.5).abs() < 1e-6);
}

#[test]
fn mpc_consolidated_partial_zero_rate_regularized_optimum() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,[.2 0],1,[0 0],.1),.1,1,1); c.Weights.MVRate=[.1 0]; s=mpcstate(c); [u,i]=mpcmove(c,s,0,1); ok=strcmp(i.QPCode,'feasible'); cost=i.Cost;").unwrap();
    // H=diag(.1,0,200000); existing documented 20*sqrt(eps) ridge.
    let ridge = 20. * f64::EPSILON.sqrt();
    let expected = 0.4 / (0.1 + ridge);
    assert_eq!(scalar(&e, "ok"), 1., "feasible semidefinite objective");
    assert!((mat(&e, "u").get(0, 0) - expected).abs() < 1e-7);
    let u = mat(&e, "u").get(0, 0);
    let expected_cost = (0.2 * u - 1.).powi(2) + 0.01 * u * u;
    assert!((scalar(&e, "cost") - expected_cost).abs() < 1e-12);
    e.execute("r=review(c); H=r.HessianMatrix.Matrix; ridge=r.Regularization; healthy=r.HessianMatrix.Pass;").unwrap();
    assert_eq!(scalar(&e, "ridge"), ridge);
    assert!((mat(&e, "H").get(0, 0) - (0.1 + ridge)).abs() < 1e-14);
    assert_eq!(mat(&e, "H").get(1, 1), ridge);
    assert_eq!(scalar(&e, "healthy"), 0.);
    eprintln!(
        "partial-zero: u={u:.16}, analytical={expected:.16}, ridge={ridge:e}, cost={:.16}",
        scalar(&e, "cost")
    );
}

#[test]
#[ignore = "release benchmark; run with RAYON_NUM_THREADS=1 and 4"]
fn benchmark_partial_zero_rate_move() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,[.2 0],1,[0 0],.1),.1,1,1); c.Weights.MVRate=[.1 0]; s=mpcstate(c); [u,i]=mpcmove(c,s,0,1);").unwrap();
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        e.execute("[u,i]=mpcmove(c,s,0,1);").unwrap();
    }
    eprintln!(
        "1000 partial-zero MPC moves with Info p=1/m=1/2 MVs: {:?}; RAYON_NUM_THREADS={:?}; limits 200 variables/4000 inequalities/128 augmented states",
        start.elapsed(),
        std::env::var("RAYON_NUM_THREADS")
    );
}
