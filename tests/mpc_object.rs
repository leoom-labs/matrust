//! Tests for the `mpc` controller value object (construction, properties,
//! nested assignment, get/set, value semantics, validation, and display).
use matrust::{Array, Engine, Matrix, engine::mpc_prediction_ss};

fn scalar(e: &Engine, name: &str) -> f64 {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m.as_scalar().unwrap(),
        other => panic!("expected numeric scalar for {name}, got {other:?}"),
    }
}

fn mat(e: &Engine, name: &str) -> Matrix {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m,
        other => panic!("expected numeric matrix for {name}, got {other:?}"),
    }
}

fn text(e: &mut Engine, expr: &str) -> String {
    e.execute(&format!("t_tmp = {expr};")).unwrap();
    match e.array("t_tmp").unwrap() {
        Array::Character(c) => c
            .to_utf8(&std::sync::atomic::AtomicBool::new(false))
            .unwrap(),
        other => panic!("expected char for {expr}, got {other:?}"),
    }
}

fn logical(e: &Engine, name: &str) -> bool {
    match e.array(name).unwrap() {
        Array::Logical(l) => l.data() == [1],
        other => panic!("expected logical scalar for {name}, got {other:?}"),
    }
}

#[test]
fn mpc_constructor_forms_and_defaults() {
    let mut e = Engine::default();
    e.execute(
        "sys_c = ss([-1 0.2; 0 -2], [1; 1], [1 0], 0);
         c1 = mpc(sys_c, 0.1);
         ts = c1.Ts;
         p = c1.PredictionHorizon;
         m = c1.ControlHorizon;
         w_ov = c1.Weights.OV;
         w_mv = c1.Weights.MV;
         w_mvr = c1.Weights.MVRate;
         w_ecr = c1.Weights.ECR;
         mv_min = c1.MV(1).Min;
         mv_max = c1.MV(1).Max;
         mv_mecr = c1.MV(1).MinECR;
         ov_mecr = c1.OV(1).MinECR;
         alg = c1.Optimizer.Algorithm;
         maxit = c1.Optimizer.MaxIterations;
         ctol = c1.Optimizer.ConstraintTolerance;
         nx0 = c1.Model.Nominal.X;
         nu0 = c1.Model.Nominal.U;
         ny0 = c1.Model.Nominal.Y;
         plant_ts = c1.Model.Plant.Ts;",
    )
    .unwrap();

    assert_eq!(scalar(&e, "ts"), 0.1);
    assert_eq!(scalar(&e, "p"), 10.0);
    assert_eq!(scalar(&e, "m"), 2.0);
    assert_eq!(scalar(&e, "w_ov"), 1.0);
    assert_eq!(scalar(&e, "w_mv"), 0.0);
    assert_eq!(scalar(&e, "w_mvr"), 0.1);
    assert_eq!(scalar(&e, "w_ecr"), 1e5);
    assert_eq!(scalar(&e, "mv_min"), f64::NEG_INFINITY);
    assert_eq!(scalar(&e, "mv_max"), f64::INFINITY);
    assert_eq!(scalar(&e, "mv_mecr"), 0.0);
    assert_eq!(scalar(&e, "ov_mecr"), 1.0);
    assert_eq!(text(&mut e, "alg"), "active-set");
    assert_eq!(scalar(&e, "maxit"), 120.0);
    assert_eq!(scalar(&e, "ctol"), 1e-6);
    assert_eq!(mat(&e, "nx0").data(), &[0.0, 0.0]);
    assert_eq!(mat(&e, "nu0").data(), &[0.0]);
    assert_eq!(mat(&e, "ny0").data(), &[0.0]);
    assert_eq!(scalar(&e, "plant_ts"), 0.0); // Model.Plant keeps continuous plant

    // Discrete plant without Ts defaults Ts to plant.Ts
    e.execute(
        "sys_d = c2d(sys_c, 0.25);
         c2 = mpc(sys_d);
         ts2 = c2.Ts;
         c3 = mpc(sys_d, 0.25, 12, 3);
         p3 = c3.PredictionHorizon;
         m3 = c3.ControlHorizon;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "ts2"), 0.25);
    assert_eq!(scalar(&e, "p3"), 12.0);
    assert_eq!(scalar(&e, "m3"), 3.0);

    // Full 8-argument positional form with [] placeholders
    e.execute(
        "W = struct('OV', 2.5, 'MVRate', 0.2);
         MV = struct('Min', -1, 'Max', 1);
         OV = struct('Min', -5, 'Max', 5);
         c4 = mpc(sys_c, 0.1, 8, [3 5], W, MV, OV, []);
         p4 = c4.PredictionHorizon;
         m4 = c4.ControlHorizon;
         w4 = c4.Weights.OutputVariables;
         mv4_min = c4.MV.Min;
         ov4_max = c4.OV.Max;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "p4"), 8.0);
    assert_eq!(mat(&e, "m4").data(), &[3.0, 5.0]);
    assert_eq!(scalar(&e, "w4"), 2.5);
    assert_eq!(scalar(&e, "mv4_min"), -1.0);
    assert_eq!(scalar(&e, "ov4_max"), 5.0);
}

#[test]
fn mpc_continuous_plant_c2d_matches_engine_c2d() {
    let mut e = Engine::default();
    for model_expr in [
        "tf([1 2], [1 3 2])",
        "zpk([-2], [-1 -3], 4)",
        "ss([-1 0.5; -0.2 -2], [1; 0.5], [1 -1], 0)",
    ] {
        e.execute(&format!(
            "sys = {model_expr};
             c = mpc(sys, 0.15);
             sysd = c2d(sys, 0.15);
             [Ad_ref, Bd_ref, Cd_ref, Dd_ref] = ssdata(sysd);"
        ))
        .unwrap();
        let (ad, bd, cd, dd, ts) = mpc_prediction_ss(&e, "c").unwrap();
        assert_eq!(ts, 0.15);
        for (got, name) in [
            (&ad, "Ad_ref"),
            (&bd, "Bd_ref"),
            (&cd, "Cd_ref"),
            (&dd, "Dd_ref"),
        ] {
            let expected = mat(&e, name);
            assert_eq!(got.rows(), expected.rows(), "{model_expr} {name} rows");
            assert_eq!(got.cols(), expected.cols(), "{model_expr} {name} cols");
            for (a, b) in got.data().iter().zip(expected.data().iter()) {
                assert!(
                    (a - b).abs() < 1e-12,
                    "{model_expr} {name} mismatch: {a} vs {b}"
                );
            }
        }
    }
}

#[test]
fn mpc_blocking_vector_validation() {
    let mut e = Engine::default();
    e.execute("sys = ss(-1, 1, 1, 0); c = mpc(sys, 0.1, 10);")
        .unwrap();
    e.execute("c.ControlHorizon = [2 3 5]; ch = c.ControlHorizon;")
        .unwrap();
    assert_eq!(mat(&e, "ch").data(), &[2.0, 3.0, 5.0]);

    for bad in [
        "c.ControlHorizon = [2 3 4];",   // sums to 9 != 10
        "c.ControlHorizon = [0 10];",    // zero entry
        "c.ControlHorizon = [-1 11];",   // negative entry
        "c.ControlHorizon = [2.5 7.5];", // non-integer
        "c.ControlHorizon = 11;",        // scalar > p
        "c.PredictionHorizon = 8;",      // breaks existing blocking sum 10
    ] {
        assert!(e.execute(bad).is_err(), "expected error for: {bad}");
    }
}

#[test]
fn mpc_property_round_trips_and_nested_set() {
    let mut e = Engine::default();
    e.execute(
        "sys = ss([-1 0; 0 -2], [1 0; 0 1], [1 0; 0 1], 0);
         c = mpc(sys, 0.1);
         c.Ts = 0.2;
         c.PredictionHorizon = 12;
         c.ControlHorizon = 4;
         c.Weights.OV = [1 0.5];
         c.Weights.MV = [0.01 0.02];
         c.Weights.MVRate = [0.2 0.3];
         c.Weights.ECR = 1e4;
         c.MV(1).Min = -2;
         c.MV(1).Max = 2;
         c.MV(1).RateMin = -0.5;
         c.MV(1).RateMax = 0.5;
         c.MV(1).MinECR = 0.1;
         c.MV(1).MaxECR = 0.2;
         c.MV(1).RateMinECR = 0.3;
         c.MV(1).RateMaxECR = 0.4;
         c.MV(1).Target = 1.5;
         c.MV(1).Name = 'u1';
         c.MV(1).Units = 'V';
         c.MV(1).ScaleFactor = 2.5;
         c.OV(2).Min = -3;
         c.OV(2).Max = 4;
         c.OV(2).MinECR = 0;
         c.OV(2).MaxECR = 2;
         c.OV(2).Name = 'y2';
         c.OV(2).Units = 'm';
         c.OV(2).ScaleFactor = 0.5;
         c.Model.Nominal.X = [0.1; -0.1];
         c.Model.Nominal.U = [1; 2];
         c.Model.Nominal.Y = [3; 4];
         c.Model.Nominal.DX = [0.01; 0.02];
         c.Optimizer.MaxIterations = 250;
         c.Optimizer.ConstraintTolerance = 1e-8;
         c.Optimizer.Algorithm = 'active-set';
         c.Notes = {'line 1', 'line 2'};
         c.UserData = [42 43];",
    )
    .unwrap();

    e.execute(
        "r_ts = c.Ts;
         r_p = c.p;
         r_m = c.m;
         r_wov = c.Weights.OutputVariables;
         r_wmv = c.Weights.ManipulatedVariables;
         r_wmvr = c.Weights.ManipulatedVariablesRate;
         r_wecr = c.Weights.ECR;
         r_mv_max = c.ManipulatedVariables(1).Max;
         r_mv_sf = c.MV(1).ScaleFactor;
         r_mv_tgt = c.MV(1).Target;
         r_ov_min = c.OutputVariables(2).Min;
         r_ov_maxecr = c.OV(2).MaxECR;
         r_nx = c.Model.Nominal.X;
         r_nu = c.Model.Nominal.U;
         r_ny = c.Model.Nominal.Y;
         r_ndx = c.Model.Nominal.DX;
         r_maxit = c.Optimizer.MaxIterations;
         r_ctol = c.Optimizer.ConstraintTolerance;
         r_ud = c.UserData;",
    )
    .unwrap();

    assert_eq!(scalar(&e, "r_ts"), 0.2);
    assert_eq!(scalar(&e, "r_p"), 12.0);
    assert_eq!(scalar(&e, "r_m"), 4.0);
    assert_eq!(mat(&e, "r_wov").data(), &[1.0, 0.5]);
    assert_eq!(mat(&e, "r_wmv").data(), &[0.01, 0.02]);
    assert_eq!(mat(&e, "r_wmvr").data(), &[0.2, 0.3]);
    assert_eq!(scalar(&e, "r_wecr"), 1e4);
    assert_eq!(scalar(&e, "r_mv_max"), 2.0);
    assert_eq!(scalar(&e, "r_mv_sf"), 2.5);
    assert_eq!(scalar(&e, "r_mv_tgt"), 1.5);
    assert_eq!(scalar(&e, "r_ov_min"), -3.0);
    assert_eq!(scalar(&e, "r_ov_maxecr"), 2.0);
    assert_eq!(mat(&e, "r_nx").data(), &[0.1, -0.1]);
    assert_eq!(mat(&e, "r_nu").data(), &[1.0, 2.0]);
    assert_eq!(mat(&e, "r_ny").data(), &[3.0, 4.0]);
    assert_eq!(mat(&e, "r_ndx").data(), &[0.01, 0.02]);
    assert_eq!(scalar(&e, "r_maxit"), 250.0);
    assert_eq!(scalar(&e, "r_ctol"), 1e-8);
    assert_eq!(mat(&e, "r_ud").data(), &[42.0, 43.0]);
    assert_eq!(text(&mut e, "c.MV(1).Name"), "u1");
    assert_eq!(text(&mut e, "c.MV(1).Units"), "V");
    assert_eq!(text(&mut e, "c.OV(2).Name"), "y2");
    assert_eq!(text(&mut e, "c.OV(2).Units"), "m");

    // Time-varying weights (p x n)
    e.execute("c.Weights.OV = ones(12, 2); r_tv = size(c.Weights.OV);")
        .unwrap();
    assert_eq!(mat(&e, "r_tv").data(), &[12.0, 2.0]);

    // Nested assignment through struct
    e.execute(
        "s.c = c;
         s.c.Weights.OV = [2 3];
         s.c.PredictionHorizon = 15;
         s.c.MV(1).Max = 9;
         s.c.Model.Nominal.X = [1; -1];
         n_ov = s.c.Weights.OV;
         n_p = s.c.PredictionHorizon;
         n_max = s.c.MV(1).Max;
         n_x = s.c.Model.Nominal.X;
         orig_p = c.PredictionHorizon;",
    )
    .unwrap();
    assert_eq!(mat(&e, "n_ov").data(), &[2.0, 3.0]);
    assert_eq!(scalar(&e, "n_p"), 15.0);
    assert_eq!(scalar(&e, "n_max"), 9.0);
    assert_eq!(mat(&e, "n_x").data(), &[1.0, -1.0]);
    assert_eq!(scalar(&e, "orig_p"), 12.0); // value semantics preserved
}

#[test]
fn mpc_weights_struct_round_trip_prefers_changed_alias() {
    let mut e = Engine::default();
    e.execute(
        "sys = ss(-1, 1, 1, 0);
         c = mpc(sys, 0.1);
         w = c.Weights;
         w.OutputVariables = 3;
         c.Weights = w;
         ov = c.Weights.OV;
         w = c.Weights;
         w.ManipulatedVariablesRate = 0.4;
         c.Weights = w;
         mvr = c.Weights.MVRate;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "ov"), 3.0);
    assert_eq!(scalar(&e, "mvr"), 0.4);
    assert!(
        e.execute(
            "w = c.Weights;
             w.OutputVariables = 4;
             w.OV = 5;
             c.Weights = w;"
        )
        .is_err()
    );
}

#[test]
fn mpc_whole_signal_struct_assignment_validates_final_bounds() {
    let mut e = Engine::default();
    e.execute(
        "sys = ss(-1, 1, 1, 0);
         c = mpc(sys, 0.1);
         c.MV(1).Max = 1;
         c.MV = struct('Min', 2, 'Max', 3);
         mv_min = c.MV.Min;
         mv_max = c.MV.Max;
         c.OV(1).Max = 1;
         c.OV = struct('Min', 2, 'Max', 3);
         ov_min = c.OV.Min;
         ov_max = c.OV.Max;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "mv_min"), 2.0);
    assert_eq!(scalar(&e, "mv_max"), 3.0);
    assert_eq!(scalar(&e, "ov_min"), 2.0);
    assert_eq!(scalar(&e, "ov_max"), 3.0);
    assert!(
        e.execute("c.MV = struct('Min', 3, 'Max', 2);").is_err(),
        "crossing MV bounds must still error"
    );
    assert!(
        e.execute("c.OV = struct('Min', 3, 'Max', 2);").is_err(),
        "crossing OV bounds must still error"
    );
}

#[test]
fn mpc_rejects_mv_direct_feedthrough() {
    let mut e = Engine::default();
    assert!(e.execute("mpc(ss(-1, 1, 1, 2), 0.1);").is_err());
    e.execute("c = mpc(ss(-1, 1, 1, 0), 0.1);").unwrap();
    assert!(e.execute("c.Model.Plant = ss(-1, 1, 1, 2);").is_err());
}

#[test]
fn mpc_value_semantics_get_set_class_isa_isequal_and_display() {
    let mut e = Engine::default();
    e.execute(
        "sys = ss(-1, 1, 1, 0);
         c1 = mpc(sys, 0.1);
         c2 = c1;
         eq_before = isequal(c1, c2);
         c2.PredictionHorizon = 20;
         p1 = c1.PredictionHorizon;
         p2 = c2.PredictionHorizon;
         eq_after = isequal(c1, c2);
         c2.PredictionHorizon = 10;
         eq_restored = isequal(c1, c2);
         is_mpc = isa(c1, 'mpc');
         is_handle = isa(c1, 'handle');
         cls = class(c1);
         g = get(c1);
         g_ts = get(c1, 'Ts');
         c3 = set(c1, 'PredictionHorizon', 14);
         p3 = c3.PredictionHorizon;
         p1_still = c1.PredictionHorizon;
         set(c1, 'ControlHorizon', 3);
         m1_after_set = c1.ControlHorizon;",
    )
    .unwrap();

    assert!(logical(&e, "eq_before"));
    assert_eq!(scalar(&e, "p1"), 10.0);
    assert_eq!(scalar(&e, "p2"), 20.0);
    assert!(!logical(&e, "eq_after"));
    assert!(logical(&e, "eq_restored"));
    assert!(logical(&e, "is_mpc"));
    assert!(!logical(&e, "is_handle"));
    assert_eq!(text(&mut e, "cls"), "mpc");
    assert_eq!(scalar(&e, "g_ts"), 0.1);
    assert_eq!(scalar(&e, "p3"), 14.0);
    assert_eq!(scalar(&e, "p1_still"), 10.0);
    assert_eq!(scalar(&e, "m1_after_set"), 3.0);

    let disp_out = e.execute("c4 = mpc(sys, 0.25, 14, 3); disp(c4)").unwrap();
    let joined = disp_out.output.join("\n");
    assert!(joined.contains("0.25"), "display missing Ts: {joined}");
    assert!(joined.contains("14"), "display missing p: {joined}");
    assert!(joined.contains("3"), "display missing m: {joined}");
    assert!(joined.contains("mpc"), "display missing class: {joined}");
}

#[test]
fn mpc_invalid_assignments_and_constructions_error() {
    let mut e = Engine::default();
    e.execute("sys = ss(-1, 1, 1, 0); sysd = c2d(sys, 0.1); c = mpc(sys, 0.1);")
        .unwrap();

    for bad in [
        "mpc()",
        "mpc(123)",
        "mpc(sys)",       // continuous without Ts
        "mpc(sys, 0)",    // Ts <= 0
        "mpc(sys, -0.1)", // Ts < 0
        "mpc(sysd, 0.2)", // discrete Ts mismatch
        "c.Ts = -1;",
        "c.PredictionHorizon = 0;",
        "c.PredictionHorizon = 2.5;",
        "c.Weights.OV = -1;",
        "c.Weights.OV = [1 2 3];",
        "c.Weights.ECR = -5;",
        "c.MV(1).Min = 5;", // wait: Max is Inf so Min=5 is valid; test Min > Max:
        "c.MV(1).Max = -10; c.MV(1).Min = 0;",
        "c.MV(1).ScaleFactor = 0;",
        "c.MV(1).MinECR = -1;",
        "c.OV(1).ScaleFactor = -2;",
        "c.Model.Nominal.X = [1 2 3];",
        "c.Optimizer.Algorithm = 'interior-point';",
        "c.Optimizer.MaxIterations = 0;",
        "c.Optimizer.ConstraintTolerance = 0;",
        "c.History = 'tamper';",
        "c.NonExistent = 1;",
    ] {
        if bad == "c.MV(1).Min = 5;" {
            continue;
        }
        assert!(e.execute(bad).is_err(), "expected error for: {bad}");
    }
}

#[test]
fn mpc_consolidated_nested_set_value_writeback() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1)); s=struct('c',c); set(s.c,'PredictionHorizon',20); p=s.c.PredictionHorizon; original=c.PredictionHorizon;").unwrap();
    assert_eq!(scalar(&e, "p"), 20.);
    assert_eq!(scalar(&e, "original"), 10.);
    e.execute("a={s}; set(a{1}.c,'PredictionHorizon',30); q=a{1}.c.PredictionHorizon; copy=set(s.c,'PredictionHorizon',40); old=s.c.PredictionHorizon; new=copy.PredictionHorizon;").unwrap();
    assert_eq!(scalar(&e, "q"), 30.);
    assert_eq!(scalar(&e, "old"), 20.);
    assert_eq!(scalar(&e, "new"), 40.);
    e.execute("key='c'; set(s.(key),'PredictionHorizon',50); set(a{end}.c,'PredictionHorizon',60); dynamic=s.c.PredictionHorizon; ending=a{1}.c.PredictionHorizon;").unwrap();
    assert_eq!(scalar(&e, "dynamic"), 50.);
    assert_eq!(scalar(&e, "ending"), 60.);
}

#[test]
fn mpc_consolidated_ecr_positive_finite_transactional() {
    let mut e = Engine::default();
    e.execute("p=ss(.8,.2,1,0,.1); c=mpc(p);").unwrap();
    for v in ["0", "-1", "NaN", "Inf"] {
        for assignment in [
            format!("c.Weights.ECR={v};"),
            format!("c.Weights=struct('ECR',{v});"),
            format!("bad=mpc(p,.1,10,2,struct('ECR',{v}));"),
        ] {
            assert!(e.execute(&assignment).is_err(), "accepted {assignment}");
            e.execute("valid=c.Weights.ECR;").unwrap();
            assert_eq!(scalar(&e, "valid"), 1e5);
        }
    }
}

#[test]
fn mpc_physical_ov_constructor_metadata_and_property_edits() {
    let mut e = Engine::default();
    e.execute("p=ss(.8,.2,[1;2],[0;0],.1); p.OutputName={'first','second'}; p=setmpcsignals(p,'MO',2,'UO',1); ov=struct('Max',{.1,100},'Units',{'m','K'},'ScaleFactor',{2,5}); c=mpc(p,.1,1,1,struct('OV',[1 0]),[],ov); b1=c.OV(1).Max; b2=c.OV(2).Max; s1=c.OV(1).ScaleFactor; s2=c.OV(2).ScaleFactor;").unwrap();
    assert_eq!(text(&mut e, "c.OV(1).Name"), "first");
    assert_eq!(text(&mut e, "c.OV(2).Name"), "second");
    assert_eq!(text(&mut e, "c.OV(1).Units"), "m");
    assert_eq!(scalar(&e, "b1"), 0.1);
    assert_eq!(scalar(&e, "b2"), 100.);
    assert_eq!(scalar(&e, "s1"), 2.);
    assert_eq!(scalar(&e, "s2"), 5.);
    e.execute("c.OV(1).Name='physical first'; box=struct('c',c); box.c.OV(2).Units='kelvin';")
        .unwrap();
    assert_eq!(text(&mut e, "box.c.OV(2).Units"), "kelvin");
    assert_eq!(text(&mut e, "c.OV(1).Name"), "physical first");
}

#[test]
fn mpc_nested_set_evaluates_side_effecting_indices_once_and_rejects_temporaries() {
    let mut e = Engine::default();
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("mpc_set_counter.m"),
        "function k=mpc_set_counter(t)\nt.LastMove=t.LastMove+1;\nk=1;\nend\n",
    )
    .unwrap();
    e.set_function_directory(folder.path()).unwrap();
    e.execute("c=mpc(ss(.8,.2,1,0,.1)); tracker=mpcstate(c); a={struct('c',c)}; set(a{mpc_set_counter(tracker)}.c,'PredictionHorizon',20); count=tracker.LastMove; horizon=a{1}.c.PredictionHorizon;").unwrap();
    assert_eq!(scalar(&e, "count"), 1.);
    assert_eq!(scalar(&e, "horizon"), 20.);
    e.execute("b=struct('c',{c,c});").unwrap();
    e.execute("set(b(2).c,'PredictionHorizon',30); horizon=b(2).c.PredictionHorizon;")
        .unwrap();
    assert_eq!(scalar(&e, "horizon"), 30.);
    assert!(
        e.execute("set(mpc(ss(.8,.2,1,0,.1)),'PredictionHorizon',20);")
            .unwrap_err()
            .contains("assignable")
    );
    let mut fresh = Engine::default();
    assert!(
        fresh
            .execute("set(mpc(ss(.8,.2,1,0,.1)),'PredictionHorizon',20);")
            .unwrap_err()
            .contains("assignable")
    );
    fresh
        .execute(
            "copy=set(mpc(ss(.8,.2,1,0,.1)),'PredictionHorizon',20); p=copy.PredictionHorizon;",
        )
        .unwrap();
    assert_eq!(scalar(&fresh, "p"), 20.);
}
