//! Independent lifted-QP / NumPy-SciPy realization from tool/refs/mpc/review.py.
use matrust::{Array, Engine, Matrix};

fn mat(e: &Engine, name: &str) -> Matrix {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m,
        a => panic!("{name}: {a:?}"),
    }
}
fn scalar(e: &Engine, name: &str) -> f64 {
    mat(e, name).data()[0]
}

#[test]
fn first_proof_scalar_spectral_radius_against_independent_numpy_realization() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/mpc/review.json")).unwrap();
    let want = data["cases"][0]["radius"].as_f64().unwrap();
    let mut e = Engine::default();
    let out = e.execute("c=mpc(ss(.8,.2,1,0,.1),.1,6,2);r=review(c);rho=r.NominalStability.MaxAbsEigenvalue;assessed=r.NominalStability.Assessed;pass=r.NominalStability.Pass;").unwrap();
    let error = (scalar(&e, "rho") - want).abs();
    eprintln!(
        "scalar max |eig|={:.16}, independent reference={want:.16}, absolute error={error:e}",
        scalar(&e, "rho")
    );
    assert!(error < 1e-6);
    assert_eq!(scalar(&e, "assessed"), 1.);
    assert_eq!(scalar(&e, "pass"), 1.);
    assert!(out.output.is_empty());
    assert!(e.figure.is_none());
}

fn compare_matrix(m: &Matrix, v: &serde_json::Value, tolerance: f64) {
    let rows = v.as_array().unwrap();
    assert_eq!(m.rows(), rows.len());
    let mut error: f64 = 0.;
    for (i, row) in rows.iter().enumerate() {
        let row = row.as_array().unwrap();
        assert_eq!(m.cols(), row.len());
        for (j, x) in row.iter().enumerate() {
            error = error.max((m.get(i, j) - x.as_f64().unwrap()).abs());
        }
    }
    assert!(
        error <= tolerance,
        "matrix maximum absolute error={error:e}"
    );
}

#[test]
fn independent_scalar_matrices_verdicts_offsets_and_exact_condensed_hessian() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/mpc/review.json")).unwrap();
    for case in data["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let a = case["a"].as_f64().unwrap();
        let b = case["b"].as_f64().unwrap();
        let mut e = Engine::default();
        let estimator = if case["gain_scale"].as_f64().unwrap() == 1. {
            String::new()
        } else {
            format!(
                "[L,M]=getEstimator(c);setEstimator(c,{}*L,M);",
                case["gain_scale"]
            )
        };
        e.execute(&format!("c=mpc(ss({a},{b},1,0,.1),.1,6,2);c.MV.ScaleFactor={};c.OV.ScaleFactor={};c.Weights.MV={};{}r=review(c);A=r.InternalStability.Matrix;Acl=r.NominalStability.Matrix;H=r.HessianMatrix.Matrix;rho=r.NominalStability.MaxAbsEigenvalue;ip=r.InternalStability.Pass;ir=r.InternalStability.MaxAbsEigenvalue;np=r.NominalStability.Pass;na=r.NominalStability.Assessed;hp=r.HessianMatrix.Pass;ha=r.HessianMatrix.Assessed;hc=r.HessianMatrix.ConditionNumber;oa=r.SteadyState.Assessed;op=r.SteadyState.Pass;offset=r.SteadyState.MaxAbsOffset;",case["su"],case["sy"],case["wu"],estimator)).unwrap();
        compare_matrix(&mat(&e, "A"), &case["internal"], 1e-8);
        compare_matrix(&mat(&e, "Acl"), &case["closed_loop"], 1e-8);
        compare_matrix(&mat(&e, "H"), &case["hessian"], 1e-8);
        let reference = case["radius"].as_f64().unwrap();
        let error = (scalar(&e, "rho") - reference).abs();
        let condition_error = (scalar(&e, "hc") / case["condition"].as_f64().unwrap() - 1.).abs();
        assert!(error < 1e-6, "{name}: radius error={error:e}");
        assert!((scalar(&e, "ir") - case["internal_radius"].as_f64().unwrap()).abs() < 1e-6);
        assert_eq!(
            scalar(&e, "ip"),
            f64::from(case["internal_radius"].as_f64().unwrap() <= 1. + 1e-9)
        );
        assert!(
            condition_error < 1e-8,
            "{name}: condition relative error={condition_error:e}"
        );
        assert_eq!(scalar(&e, "np"), f64::from(reference < 1. - 1e-9));
        for k in ["na", "ha", "hp"] {
            assert_eq!(scalar(&e, k), 1., "{name}: {k}");
        }
        if let Some(expected) = case["offset"].as_f64() {
            assert_eq!(scalar(&e, "oa"), 1.);
            assert!(
                (scalar(&e, "offset") - expected.abs()).abs() < 1e-6,
                "{name}: offset"
            );
            assert_eq!(scalar(&e, "op"), f64::from(expected.abs() <= 1e-6));
        } else {
            assert_eq!(scalar(&e, "oa"), 0.);
            e.execute("reason=contains(r.SteadyState.Reason,'not strictly stable');")
                .unwrap();
            assert_eq!(logical(&e, "reason"), 1.);
        }
        eprintln!(
            "{name}: radius absolute error={error:e}, condition relative error={condition_error:e}, offset={}",
            scalar(&e, "offset")
        );
    }
}

fn logical(e: &Engine, name: &str) -> f64 {
    match e.array(name).unwrap() {
        Array::Logical(m) => f64::from(m.data()[0]),
        Array::Numeric(m) => m.data()[0],
        a => panic!("{name}: {a:?}"),
    }
}

#[test]
fn text_dispatch_agrees_with_struct_and_repeated_review_preserves_controller_state() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1),.1,6,2);c.MV.Min=-.6;c.MV.Max=.6;c.MV.RateMin=-.1;c.MV.RateMax=.1;d=c;s=mpcstate(c);t=mpcstate(d);[u,i]=mpcmove(c,s,.2,1);[v,j]=mpcmove(d,t,.2,1);p=s.Plant;q=s.Disturbance;n=s.Noise;l=s.LastMove;cov=s.Covariance;[L0,M0]=getEstimator(c);r=review(c);for k=1:12;rr=review(c);end;same=isequal(c,d);state=isequal(s.Plant,p)&&isequal(s.Disturbance,q)&&isequal(s.Noise,n)&&isequal(s.LastMove,l)&&isequal(s.Covariance,cov);[L1,M1]=getEstimator(c);gains=isequal(L0,L1)&&isequal(M0,M1);textsame=strcmp(r.Text,rr.Text);[u,i]=mpcmove(c,s,.3,1);[v,j]=mpcmove(d,t,.3,1);rho=r.NominalStability.MaxAbsEigenvalue;offset=r.SteadyState.MaxAbsOffset;tt=r.Text;").unwrap();
    for key in ["same", "state", "gains", "textsame"] {
        assert_eq!(logical(&e, key), 1., "{key}");
    }
    assert_eq!(mat(&e, "u").data(), mat(&e, "v").data());
    let out = e.execute("review(c);").unwrap();
    assert_eq!(out.output.len(), 1);
    let text = &out.output[0];
    assert!(text.contains(&format!("max |eig|={:.12e}", scalar(&e, "rho"))));
    assert!(text.contains(&format!("max |offset|={:.12e}", scalar(&e, "offset"))));
    assert!(text.contains("Ts=") && text.contains("MV(1)") && text.contains("Weights OV="));
    let Array::Character(tt) = e.array("tt").unwrap() else {
        panic!()
    };
    assert_eq!(
        text,
        &tt.to_utf8(&std::sync::atomic::AtomicBool::new(false))
            .unwrap()
    );
    assert!(e.array("ans").is_none());
    assert!(e.figure.is_none());
    assert!(e.execute("report=review(c);").unwrap().output.is_empty());
    for call in ["review()", "review(1)", "review(c,c)", "[a,b]=review(c)"] {
        assert!(e.execute(call).unwrap_err().contains("review"));
    }
}

#[test]
fn singular_ill_conditioned_and_regularized_hessians_are_never_healthy() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,[.2 0],1,[0 0],.1),.1,6,2);c.Weights.MVRate=[.1 0];r=review(c);a=r.HessianMatrix.Assessed;p=r.HessianMatrix.Pass;h=r.HessianMatrix.ConditionNumber;reg=r.Regularization;").unwrap();
    assert_eq!(scalar(&e, "a"), 1.);
    assert_eq!(scalar(&e, "p"), 0.);
    assert_eq!(scalar(&e, "reg"), 20. * f64::EPSILON.sqrt());
    assert!(scalar(&e, "h").is_finite());
    e.execute("c=mpc(ss(.8,.2,1,0,.1),.1,6,2);c.Weights.ECR=1e20;r=review(c);a=r.HessianMatrix.Assessed;p=r.HessianMatrix.Pass;h=r.HessianMatrix.ConditionNumber;").unwrap();
    assert_eq!(scalar(&e, "a"), 1.);
    assert_eq!(scalar(&e, "p"), 0.);
    assert!(scalar(&e, "h") > 1e12);
    e.execute("c=mpc(ss(.8,.2,1,0,.1),.1,6,2);c.Weights.OV=0;c.Weights.MVRate=0;r=review(c);p=r.HessianMatrix.Pass;reg=r.Regularization;H=r.HessianMatrix.Matrix;").unwrap();
    assert_eq!(scalar(&e, "p"), 0.);
    assert!(scalar(&e, "reg") > 0.);
    assert_eq!(mat(&e, "H").get(0, 0), scalar(&e, "reg"));
}

#[test]
fn concrete_hard_constraint_conflicts_and_ordinary_finite_bounds() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1),.1,6,2);c.MV.Min=1;c.MV.RateMax=0;r=review(c);a=r.HardConstraints.Assessed;p=r.HardConstraints.Pass;n=r.HardConstraints.IssueCount;channel=contains(r.HardConstraints.Details,'MV(1) step 1');issue=contains(r.HardConstraints.Details,'contradictory');").unwrap();
    assert_eq!(scalar(&e, "a"), 1.);
    assert_eq!(scalar(&e, "p"), 0.);
    assert_eq!(scalar(&e, "n"), 1.);
    assert_eq!(logical(&e, "channel"), 1.);
    assert_eq!(logical(&e, "issue"), 1.);
    e.execute("c=mpc(ss(.8,.2,1,0,.1),.1,6,2);c.MV.Min=-1;c.MV.Max=1;c.MV.RateMin=-.1;c.MV.RateMax=.1;c.OV.Min=-2;c.OV.Max=2;c.OV.MinECR=0;c.OV.MaxECR=0;r=review(c);p=r.HardConstraints.Pass;n=r.HardConstraints.IssueCount;finite=r.HardConstraints.FiniteBoundEntries;").unwrap();
    assert_eq!(scalar(&e, "p"), 1.);
    assert_eq!(scalar(&e, "n"), 0.);
    assert_eq!(scalar(&e, "finite"), 6.);
    // Held moves after m=2 cannot satisfy a strictly positive hard rate.
    e.execute("c.MV.RateMin=.01;r=review(c);p=r.HardConstraints.Pass;held=contains(r.HardConstraints.Details,'step 3');").unwrap();
    assert_eq!(scalar(&e, "p"), 0.);
    assert_eq!(logical(&e, "held"), 1.);
    // A soft rate can be relaxed, so it does not cause a hard contradiction.
    e.execute("c.MV.RateMinECR=1;r=review(c);p=r.HardConstraints.Pass;")
        .unwrap();
    assert_eq!(scalar(&e, "p"), 1.);
}

#[test]
fn unsupported_assumptions_have_explicit_reasons_but_keep_actual_hessian() {
    for (setup, reason) in [
        ("c.ControlHorizon=[2 4];", "blocking-vector"),
        ("c.Weights.OV=ones(6,1);", "time-varying"),
        ("setEstimator(c,'custom');", "custom estimator"),
    ] {
        let mut e = Engine::default();
        e.execute(&format!("c=mpc(ss(.8,.2,1,0,.1),.1,6,2);{setup}d=c;r=review(c);a=r.NominalStability.Assessed;o=r.SteadyState.Assessed;h=r.HessianMatrix.Assessed;same=isequal(c,d);why=contains(r.NominalStability.Reason,'{reason}');")).unwrap();
        assert_eq!(scalar(&e, "a"), 0.);
        assert_eq!(scalar(&e, "o"), 0.);
        assert_eq!(scalar(&e, "h"), 1.);
        assert_eq!(logical(&e, "same"), 1.);
        assert_eq!(logical(&e, "why"), 1.);
    }
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1),.1,129,2);r=review(c);a=r.NominalStability.Assessed;why=contains(r.NominalStability.Reason,'analysis limit');").unwrap();
    assert_eq!(scalar(&e, "a"), 0.);
    assert_eq!(logical(&e, "why"), 1.);
}

#[test]
fn nominal_offsets_and_scaling_leave_the_expected_deviation_dynamics() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/mpc/review.json")).unwrap();
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1),.1,6,2);c.MV.ScaleFactor=2;c.OV.ScaleFactor=4;c.Model.Nominal=struct('X',2,'U',3,'Y',5,'DX',.06);r=review(c);A=r.NominalStability.Matrix;offset=r.SteadyState.MaxAbsOffset;baseline=r.BaselineError;a=r.SteadyState.Assessed;").unwrap();
    compare_matrix(&mat(&e, "A"), &data["cases"][3]["closed_loop"], 1e-8);
    assert_eq!(scalar(&e, "a"), 1.);
    assert!(scalar(&e, "offset") < 1e-6);
    assert!(scalar(&e, "baseline").abs() < 1e-6);
}

#[test]
fn independent_mimo_with_mo_uo_md_ud_nominal_offsets_and_scales() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/mpc/review.json")).unwrap();
    let case = &data["partitioned"];
    let mut e = Engine::default();
    e.execute(case["source"].as_str().unwrap()).unwrap();
    e.execute("r=review(c);A=r.InternalStability.Matrix;Acl=r.NominalStability.Matrix;H=r.HessianMatrix.Matrix;E=r.SteadyState.Matrix;b=r.BaselineError;a=r.SteadyState.Assessed;rho=r.NominalStability.MaxAbsEigenvalue;").unwrap();
    assert_eq!(scalar(&e, "a"), 1.);
    for (key, fixture) in [
        ("A", "internal"),
        ("Acl", "closed_loop"),
        ("H", "hessian"),
        ("E", "error"),
        ("b", "baseline"),
    ] {
        compare_matrix(&mat(&e, key), &case[fixture], 1e-7);
    }
    assert!((scalar(&e, "rho") - case["radius"].as_f64().unwrap()).abs() < 1e-6);
}

#[test]
fn cancellation_memory_and_output_limits_are_enforced() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1));").unwrap();
    let mut limits = e.limits_active();
    limits.memory_bytes = 300_000;
    e.limits_set(limits);
    e.execute("r=review(c);a=r.NominalStability.Assessed;why=contains(r.NominalStability.Reason,'memory limit');").unwrap();
    assert_eq!(scalar(&e, "a"), 0.);
    assert_eq!(logical(&e, "why"), 1.);
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1));").unwrap();
    let mut limits = e.limits_active();
    limits.display_bytes = 512;
    e.limits_set(limits);
    assert!(
        e.execute("review(c);")
            .unwrap_err()
            .contains("output limit")
    );
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1));").unwrap();
    e.cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(e.execute("r=review(c);").is_err());
    assert!(e.array("r").is_none());
}

#[test]
#[ignore = "release benchmark; run with RAYON_NUM_THREADS=1 and 4"]
fn benchmark_read_only_review() {
    for (p, m, repeats) in [(6, 2, 200), (40, 12, 30)] {
        let mut e = Engine::default();
        e.execute(&format!("c=mpc(ss([.8 .1;0 .7],[.2 .05;.1 .3],eye(2),zeros(2),.1),.1,{p},{m});r=review(c);a=r.NominalStability.Assessed;")).unwrap();
        assert_eq!(scalar(&e, "a"), 1.);
        let start = std::time::Instant::now();
        for _ in 0..repeats {
            e.execute("r=review(c);").unwrap();
        }
        eprintln!(
            "{repeats} reviews, 2 plant states/MVs/outputs, p={p}, m={m}: {:?}; RAYON_NUM_THREADS={:?}; analysis limits 32 states/16 channels/128 horizon/64 QP variables",
            start.elapsed(),
            std::env::var("RAYON_NUM_THREADS")
        );
    }
}

#[test]
fn mpc_physical_output_weight_and_scale_enter_shared_review_objective() {
    let mut e = Engine::default();
    e.execute("p=setmpcsignals(ss(.8,.2,[1;2],[0;0],.1),'MO',2,'UO',1); c=mpc(p,.1,1,1); c.Weights.OV=[1 0]; c.OV(1).ScaleFactor=2; c.OV(2).ScaleFactor=5; r=review(c); H=r.HessianMatrix.Matrix;").unwrap();
    // 2*((.2/2)^2 + .1^2), physical output 1 only.
    assert!((mat(&e, "H").get(0, 0) - 0.04).abs() < 1e-12);
}
