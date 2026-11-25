//! Independent NumPy/SciPy fixtures; regenerate with tool/refs/mpc/estimator.py.
use matrust::{Array, Engine, Matrix, engine::mpc_prediction_model};
use serde_json::Value;

fn mat(e: &Engine, name: &str) -> Matrix {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m,
        other => panic!("{name}: {other:?}"),
    }
}
fn reference(m: &Matrix, v: &Value, label: &str) -> f64 {
    let rows = v.as_array().unwrap();
    assert_eq!(m.rows(), rows.len(), "{label} rows");
    let mut error: f64 = 0.;
    if let Some(row) = rows.first() {
        assert_eq!(m.cols(), row.as_array().unwrap().len(), "{label} cols");
    }
    for (i, row) in rows.iter().enumerate() {
        for (j, x) in row.as_array().unwrap().iter().enumerate() {
            error = error.max((m.get(i, j) - x.as_f64().unwrap()).abs());
        }
    }
    assert!(error < 1e-8, "{label}: maximum absolute error {error:e}");
    error
}
fn vector(m: &Matrix, v: &Value, label: &str) {
    let v = v.as_array().unwrap();
    assert_eq!(m.len(), v.len(), "{label}");
    for (got, want) in m.data().iter().zip(v) {
        assert!((got - want.as_f64().unwrap()).abs() < 1e-12, "{label}");
    }
}
fn indices(v: &Value) -> Vec<usize> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_u64().unwrap() as usize)
        .collect()
}
#[test]
fn scipy_augmented_models_gains_and_nominal_state() {
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/mpc/estimator.json")).unwrap();
    let mut max_l: f64 = 0.;
    let mut max_m: f64 = 0.;
    for case in fixtures["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut e = Engine::default();
        e.execute(case["source"].as_str().unwrap())
            .unwrap_or_else(|err| panic!("{name}: {err}"));
        e.execute("[L,M,A,Cm,Bu,Bv,Dvm]=getEstimator(c);s=mpcstate(c);sp=s.Plant;sd=s.Disturbance;sn=s.Noise;su=s.LastMove;P=s.Covariance;").unwrap_or_else(|err|panic!("{name}: {err}"));
        let p = mpc_prediction_model(&mut e, "c").unwrap();
        for (key, m) in [
            ("A", &p.a),
            ("Bu", &p.bu),
            ("Bv", &p.bv),
            ("Bd", &p.bd),
            ("C", &p.c),
            ("Cprediction", &p.c_prediction),
            ("Dv", &p.dv),
            ("Dd", &p.dd),
            ("Cm", &p.cm),
            ("Dvm", &p.dvm),
        ] {
            reference(m, &case[key], &format!("{name} {key}"));
        }
        for key in ["A", "Cm", "Bu", "Bv", "Dvm", "P"] {
            reference(&mat(&e, key), &case[key], &format!("{name} {key}"));
        }
        max_l = max_l.max(reference(&mat(&e, "L"), &case["L"], &format!("{name} L")));
        max_m = max_m.max(reference(&mat(&e, "M"), &case["M"], &format!("{name} M")));
        assert_eq!(
            p.input_disturbance_states.len(),
            case["ni"].as_u64().unwrap() as usize
        );
        assert_eq!(
            p.output_disturbance_states.len(),
            case["no"].as_u64().unwrap() as usize
        );
        assert_eq!(p.noise_states.len(), case["nn"].as_u64().unwrap() as usize);
        for (key, actual) in [
            ("mv", &p.mv_idx),
            ("md", &p.md_idx),
            ("ud", &p.ud_idx),
            ("mo", &p.mo_idx),
            ("uo", &p.uo_idx),
            ("ich", &p.input_integrator_channels),
            ("och", &p.output_integrator_channels),
        ] {
            assert_eq!(*actual, indices(&case[key]), "{name} {key}");
        }
        vector(&p.x_offset, &case["X"], name);
        vector(&p.u_offset, &case["U"], name);
        vector(&p.y_offset, &case["Y"], name);
        vector(&mat(&e, "sp"), &case["X"], name);
        assert_eq!(
            mat(&e, "sd").len(),
            p.input_disturbance_states.len() + p.output_disturbance_states.len()
        );
        assert_eq!(mat(&e, "sn").len(), p.noise_states.len());
        assert!(
            mat(&e, "sd")
                .data()
                .iter()
                .chain(mat(&e, "sn").data())
                .all(|&x| x == 0.)
        );
        let last = mat(&e, "su");
        for (j, &i) in p.mv_idx.iter().enumerate() {
            assert_eq!(last.get(j, 0), p.u_offset.get(i, 0));
        }
        assert_eq!(p.bv.cols(), p.md_idx.len() + 1);
    }
    eprintln!("maximum L error={max_l:e}; maximum M error={max_m:e}");
}
#[test]
fn estimator_round_trip_defaults_custom_and_aliases() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(0.8,0.2,1,0,0.1));[L0,M0,A]=getEstimator(c);alias=c;L1=[0.1;0.2];M1=[0.3;0.4];setEstimator(c,L1,M1);[L,M]=getEstimator(c);[La,Ma]=getEstimator(alias);").unwrap();
    assert_eq!(mat(&e, "L").data(), mat(&e, "L1").data());
    assert_eq!(mat(&e, "M").data(), mat(&e, "M1").data());
    assert_eq!(mat(&e, "La").data(), mat(&e, "L0").data());
    e.execute("od=getoutdist(c);[odA,odB,odC,odD]=ssdata(od);")
        .unwrap();
    assert_eq!(mat(&e, "odB").data(), &[0.1]);
    e.execute("setEstimator(c,[],M1);[L,M]=getEstimator(c);AM=A*M1;")
        .unwrap();
    assert_eq!(mat(&e, "L").data(), mat(&e, "AM").data());
    e.execute("setEstimator(c,L1);[L,M]=getEstimator(c);")
        .unwrap();
    assert_eq!(mat(&e, "M").data(), &[0., 0.]);
    e.execute("setEstimator(c,'default');[L,M]=getEstimator(c);")
        .unwrap();
    for key in ["L", "M"] {
        assert_eq!(mat(&e, key).data(), mat(&e, &format!("{key}0")).data());
    }
    e.execute("setEstimator(c,'custom');[L,M,A,Cm,Bu,Bv,Dvm]=getEstimator(c);s=mpcstate(c);P=s.Covariance;").unwrap();
    assert!(mat(&e, "L").is_empty() && mat(&e, "M").is_empty() && mat(&e, "P").is_empty());
    for bad in [
        "setEstimator(c,ones(3,1),ones(3,1))",
        "setEstimator(c,NaN,[])",
        "setEstimator(c,'typo')",
        "[a,b,c,d,e,f,g,h]=getEstimator(c)",
        "getEstimator(c,'sys')",
    ] {
        assert!(e.execute(bad).is_err(), "{bad}");
    }
}
#[test]
fn disturbance_forms_dimensions_and_reset() {
    let mut e = Engine::default();
    e.execute("p=ss(0.8,[0.2 0.4],1,[0 0],0.1);p=setmpcsignals(p,'MV',1,'UD',2);c=mpc(p);[id,ich]=getindist(c);[od,och]=getoutdist(c);[ia,ib,ic,idm]=ssdata(id);").unwrap();
    assert_eq!(mat(&e, "ich").data(), &[1.]);
    assert_eq!(mat(&e, "ib").data(), &[0.1]);
    assert!(mat(&e, "och").is_empty());
    assert!(mat(&e, "od").is_empty());
    e.execute("setoutdist(c,'model',2);setindist(c,'model',0.5);[id,ich]=getindist(c);[od,och]=getoutdist(c);[ia,ib,ic,idm]=ssdata(id);[oa,ob,oc,odm]=ssdata(od);[L,M]=getEstimator(c);").unwrap();
    assert!(mat(&e, "ia").is_empty() && mat(&e, "oa").is_empty());
    assert_eq!(mat(&e, "idm").data(), &[0.5]);
    assert_eq!(mat(&e, "odm").data(), &[2.]);
    assert!(mat(&e, "ich").is_empty() && mat(&e, "och").is_empty());
    e.execute("setoutdist(c,'model',[]);setindist(c,'integrators');[id,ich]=getindist(c);[od,och]=getoutdist(c);").unwrap();
    assert_eq!(mat(&e, "ich").data(), &[1.]);
    for bad in [
        "c.Model.Noise=ss(0.5,1,[1;1],[0;0],0.1)",
        "c.Model.Disturbance=ss(0.5,1,[1;1],[0;0],0.1)",
        "c.Model.Noise=[1;2]",
        "c.Model.Disturbance=[1;2]",
        "c.Model.Noise=ss(0.5,1,1,1,0.2)",
        "setoutdist(c,'model',[1;2])",
        "setindist(c,'model',[1;2])",
        "setoutdist(c,'typo',1)",
    ] {
        assert!(e.execute(bad).is_err(), "{bad}");
    }
    e.execute("c.Model.Noise=2;c.Model.Disturbance=[];setoutdist(c,'model',ss(-1,1,1,0));[od,ch]=getoutdist(c);[a,b,cc,d]=ssdata(od);").unwrap();
    assert!((mat(&e, "a").data()[0] - (-0.1f64).exp()).abs() < 1e-12);
}
#[test]
fn state_handle_validation_initializers_and_display() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss([0.8 0.1;0 0.7],[0.2;0.1],[1 0],0,0.1));c.Model.Nominal.X=[2;3];c.Model.Nominal.U=4;s=mpcstate(c);copy=s;s.Plant=[5 6];s.Disturbance=0.3;s.LastMove=7;v=copy.Plant;lm=copy.LastMove;h=isa(s,'handle');k=isa(s,'mpcstate');set(s,'Plant',[8;9]);v2=get(copy,'Plant');box=struct('s',s);box.s.LastMove=8;v3=s.LastMove;props=fieldnames(s);").unwrap();
    assert_eq!(mat(&e, "v").data(), &[5., 6.]);
    assert_eq!(mat(&e, "v2").data(), &[8., 9.]);
    assert_eq!(mat(&e, "v3").data(), &[8.]);
    for key in ["h", "k"] {
        assert!(matches!(e.array(key).unwrap(),Array::Logical(m) if m.data()==[1]));
    }
    let displayed = e.execute("disp(s)").unwrap().output.join("\n");
    assert!(displayed.contains("MPCSTATE") && displayed.contains("Covariance"));
    for bad in [
        "s.Plant=1",
        "s.Plant=ones(2)",
        "s.Disturbance=[1;2]",
        "s.Noise=1",
        "s.LastMove=[1;2]",
        "s.Covariance=eye(2)",
        "s.Covariance=[1 2 0;0 1 0;0 0 1]",
        "s.Plant=[NaN;0]",
        "s.Typo=1",
        "mpcstate(c,ones(3,1))",
    ] {
        assert!(e.execute(bad).is_err(), "{bad}");
    }
    e.execute("s2=mpcstate(c,[10;11],0.4,[],12,eye(3));p=s2.Plant;lm=s2.LastMove;")
        .unwrap();
    assert_eq!(mat(&e, "p").data(), &[10., 11.]);
    assert_eq!(mat(&e, "lm").data(), &[12.]);
}
#[test]
fn observability_weight_order_and_custom_mode() {
    let mut e = Engine::default();
    e.execute(
        "c=mpc(ss(1,[0.2 0.3],[1;1],zeros(2),0.1));c.Weights.OV=[1 3];[od,ch]=getoutdist(c);",
    )
    .unwrap();
    assert_eq!(mat(&e, "ch").data(), &[2.]);
    e.execute("c.Weights.OV=[3 1;0 5;0 0;0 0;0 0;0 0;0 0;0 0;0 0;0 0];[od,ch]=getoutdist(c);")
        .unwrap();
    assert_eq!(mat(&e, "ch").data(), &[2.]);
    e.execute("c.Weights.OV=[1 1];[od,ch]=getoutdist(c);")
        .unwrap();
    assert_eq!(mat(&e, "ch").data(), &[1.]);
    e.execute("setoutdist(c,'model',ss(eye(2),eye(2),eye(2),zeros(2),0.1));")
        .unwrap();
    assert!(e.execute("getEstimator(c)").is_err());
    e.execute("setEstimator(c,'custom');s=mpcstate(c);p=s.Covariance;")
        .unwrap();
    assert!(mat(&e, "p").is_empty());
}
#[test]
fn prediction_limits_and_cancellation() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(0.8,0.2,1,0,0.1));").unwrap();
    let mut limits = e.limits_active();
    limits.memory_bytes = 50_000;
    e.limits_set(limits);
    assert!(
        mpc_prediction_model(&mut e, "c")
            .unwrap_err()
            .contains("memory")
    );
    let mut e = Engine::default();
    e.execute("c=mpc(ss(0.8,0.2,1,0,0.1));").unwrap();
    e.cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(mpc_prediction_model(&mut e, "c").is_err());
}
#[test]
#[ignore = "release benchmark; run with RAYON_NUM_THREADS=1 and 4"]
fn estimator_release_benchmark() {
    for (plant_states, repeats) in [(2, 100), (16, 20)] {
        let mut e = Engine::default();
        e.execute(&format!("c=mpc(ss(0.8*eye({plant_states}),0.2*eye({plant_states}),eye({plant_states}),zeros({plant_states}),0.1));")).unwrap();
        // Warm the shared kernels and the worker pool before measuring.
        e.execute("[L,M]=getEstimator(c);").unwrap();
        let start = std::time::Instant::now();
        for _ in 0..repeats {
            e.execute("[L,M]=getEstimator(c);").unwrap();
        }
        eprintln!(
            "{repeats} augmented-model+DARE designs; plant states={plant_states}, augmented states={}: {:?}; RAYON_NUM_THREADS={:?}; limits: 128 states/inputs/outputs",
            2 * plant_states,
            start.elapsed(),
            std::env::var("RAYON_NUM_THREADS")
        );
    }
}

#[test]
fn signal_partitions_reordering_and_validation() {
    let mut e = Engine::default();
    e.execute("p=ss([0.8 0.1;0.05 0.7],[0.2 0;0.1 0.3],eye(2),zeros(2),0.1);p=setmpcsignals(p,'UO',1,'MD',1);c=mpc(p);[L,M,A,Cm,Bu,Bv,Dvm]=getEstimator(c);").unwrap();
    let model = mpc_prediction_model(&mut e, "c").unwrap();
    assert_eq!(model.mo_idx, vec![1]);
    assert_eq!(model.uo_idx, vec![0]);
    assert_eq!(model.mv_idx, vec![1]);
    assert_eq!(model.md_idx, vec![0]);
    assert_eq!(model.c.get(0, 0), 0.);
    assert_eq!(model.c.get(0, 1), 1.);
    assert_eq!(model.c.get(1, 0), 1.);
    assert_eq!(model.c.get(1, 1), 0.);
    for bad in [
        "setmpcsignals(p,'MD',3)",
        "setmpcsignals(p,'MD',0)",
        "setmpcsignals(p,'MV',1,'MD',1)",
        "setmpcsignals(p,'UO',[1 1])",
        "setmpcsignals(p,'MD',NaN)",
        "setmpcsignals(p,'typo',1)",
        "setmpcsignals(p,'MD')",
        "setmpcsignals(p,'MD',1,'MeasuredDisturbances',2)",
        "setmpcsignals()",
    ] {
        assert!(e.execute(bad).is_err(), "{bad}");
    }
    e.execute("p=setmpcsignals(p);c=mpc(p);").unwrap();
    let model = mpc_prediction_model(&mut e, "c").unwrap();
    assert_eq!(model.mo_idx, vec![0, 1]);
    assert_eq!(model.mv_idx, vec![0, 1]);
}

#[test]
fn unchanged_properties_preserve_gains_but_model_and_scale_changes_reset() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1)); setEstimator(c,[0;0],[0;0]); c.Ts=.1; c.Weights.OV=1; c.OV.ScaleFactor=1; c.OV.Max=1; c.OV.Name='plant output'; [L,M]=getEstimator(c);").unwrap();
    assert_eq!(mat(&e, "L").data(), &[0., 0.]);
    assert_eq!(mat(&e, "M").data(), &[0., 0.]);
    e.execute("c.OV.ScaleFactor=2; [L,M]=getEstimator(c);")
        .unwrap();
    assert!(mat(&e, "L").data().iter().any(|x| *x != 0.));
    e.execute("setEstimator(c,[0;0],[0;0]); c.Model.Plant=ss([.8 0;0 .7],[.2;.1],[1 1],0,.1); [L,M]=getEstimator(c);").unwrap();
    assert_eq!(mat(&e, "L").rows(), 3);
    assert_eq!(mat(&e, "M").rows(), 3);
}
