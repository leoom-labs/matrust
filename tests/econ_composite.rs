//! NumPy/SciPy reference: tool/refs/econ/composite.py; public MathWorks arima
//! composite-model documentation, without executing MATLAB.
use matrust::{Array, Engine, Matrix};
use serde_json::Value as Json;

fn run(e: &mut Engine, source: &str) {
    e.execute(source)
        .unwrap_or_else(|error| panic!("{source}: {error}"));
}
fn check(e: &mut Engine, source: &str) {
    run(e, &format!("assert({source});"));
}
fn fail(e: &mut Engine, source: &str, message: &str) {
    let error = e.execute(source).unwrap_err();
    assert!(error.contains(message), "{source}: {error}");
}
fn values(e: &Engine, name: &str) -> Vec<f64> {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        a => panic!("{name}: {a:?}"),
    }
}
fn nums(v: &Json) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}
fn near(got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len());
    for (i, (&a, &b)) in got.iter().zip(want).enumerate() {
        assert!((a - b).abs() <= 1e-10, "[{i}] {a:.16e} != {b:.16e}");
    }
}
fn refs() -> Json {
    serde_json::from_str(include_str!("fixtures/econ/composite.json")).unwrap()
}
fn vector(v: &Json) -> String {
    format!(
        "[{}]",
        nums(v)
            .iter()
            .map(|v| format!("{v:.17e}"))
            .collect::<Vec<_>>()
            .join(" ")
    )
}
fn model(c: &Json) -> String {
    let m = &c["model"];
    let vol_vector = |v: &Json| vector(v).replace('[', "{").replace(']', "}");
    let mut vol = format!(
        "{}('Constant',{},'GARCH',{},'ARCH',{}",
        m["kind"].as_str().unwrap(),
        m["omega"],
        vol_vector(&m["garch"]),
        vol_vector(&m["arch"])
    );
    if !nums(&m["leverage"]).is_empty() {
        vol.push_str(&format!(",'Leverage',{}", vol_vector(&m["leverage"])));
    }
    vol.push(')');
    let mut result = format!(
        "arima('Constant',{},'AR',{},'MA',{},'D',{},'SAR',{},'Variance',{}",
        m["c"],
        vector(&m["ar"]),
        vector(&m["ma"]),
        m["d"],
        vector(&m["sar"]),
        vol
    );
    if !m["nu"].is_null() {
        result.push_str(&format!(
            ",'Distribution',struct('Name','t','DoF',{})",
            m["nu"]
        ));
    }
    result.push(')');
    result
}
fn insert(e: &mut Engine, name: &str, data: Vec<f64>, cols: usize) {
    e.insert_array(
        name,
        Array::Numeric(Matrix::from_column_vec(data.len() / cols, cols, data).unwrap()),
    )
    .unwrap();
}
fn sample(e: &mut Engine, c: &Json) {
    for key in ["y", "y0", "e0", "v0"] {
        insert(e, key, nums(&c[key]), 1);
    }
    run(e, &format!("m={};", model(c)));
}

#[test]
fn fixed_infer_gaussian_garch_student_gjr_and_egarch_against_scipy() {
    for c in refs()["cases"].as_array().unwrap() {
        let mut e = Engine::default();
        sample(&mut e, c);
        run(&mut e, "[z,v,ll]=infer(m,y,Y0=y0,E0=e0,V0=v0);");
        near(&values(&e, "z"), &nums(&c["e"]));
        near(&values(&e, "v"), &nums(&c["v"]));
        near(&values(&e, "ll"), &[c["logl"].as_f64().unwrap()]);
        // arima.infer Name-Value E0: zero. V0: residual second moment.
        run(
            &mut e,
            "[z1,v1]=infer(m,y,Y0=y0,E0=e0);[z2,v2]=infer(m,y,Y0=y0);",
        );
        near(&values(&e, "z1"), &nums(&c["e"]));
        near(&values(&e, "v1"), &nums(&c["default_v"]));
        near(&values(&e, "z2"), &nums(&c["default_e"]));
        near(&values(&e, "v2"), &nums(&c["default_all_v"]));
    }
}

#[test]
fn fixed_forecast_mean_variance_and_mse_against_numpy_convolution() {
    for c in refs()["cases"].as_array().unwrap() {
        let mut e = Engine::default();
        sample(&mut e, c);
        run(
            &mut e,
            "[z,v]=infer(m,y,Y0=y0,E0=e0,V0=v0);[f,s,vf]=forecast(m,16,y,E0=z,V0=v);",
        );
        for (name, key) in [("f", "y"), ("s", "mse"), ("vf", "v")] {
            near(&values(&e, name), &nums(&c["forecast"][key]));
        }
        run(&mut e, "[f,s,vf]=forecast(m,16,'Y0',y,E0=z);");
        for (name, key) in [("f", "y"), ("s", "mse"), ("vf", "v")] {
            near(&values(&e, name), &nums(&c["forecast_infer_v"][key]));
        }
        run(&mut e, "[f,s,vf]=forecast(m,16,y);");
        for (name, key) in [("f", "y"), ("s", "mse"), ("vf", "v")] {
            near(&values(&e, name), &nums(&c["forecast_infer_all"][key]));
        }
        // Short E0 cannot infer V0: unconditional variance is the fallback.
        let p = nums(&c["model"]["a"]).len() - 1;
        let q = nums(&c["model"]["b"]).len() - 1;
        let q = q
            .max(nums(&c["model"]["arch"]).len())
            .max(nums(&c["model"]["leverage"]).len());
        let y = nums(&c["y"]);
        insert(
            &mut e,
            "short",
            if p == 0 {
                vec![0.]
            } else {
                y[y.len() - p..].to_vec()
            },
            1,
        );
        run(
            &mut e,
            &format!("[f,s,vf]=forecast(m,16,short,E0=zeros({q},1));"),
        );
        for (name, key) in [("f", "y"), ("s", "mse"), ("vf", "v")] {
            near(&values(&e, name), &nums(&c["forecast_short"][key]));
        }
    }
}

#[test]
fn simulation_seed_reproducibility_and_infer_round_trip_all_families() {
    for c in refs()["cases"].as_array().unwrap() {
        let mut e = Engine::default();
        sample(&mut e, c);
        run(
            &mut e,
            "rng(971);[a,z,v]=simulate(m,50,NumPaths=3,Y0=y0,E0=e0,V0=v0);rng(971);[b,z2,v2]=simulate(m,50,NumPaths=3,Y0=y0,E0=e0,V0=v0);",
        );
        check(
            &mut e,
            "isequal(a,b) && isequal(z,z2) && isequal(v,v2) && all(all(v>0))",
        );
        run(&mut e, "[zi,vi,ll]=infer(m,a,Y0=y0,E0=e0,V0=v0);");
        near(&values(&e, "zi"), &values(&e, "z"));
        near(&values(&e, "vi"), &values(&e, "v"));
        run(
            &mut e,
            "rng(39);[a,z,v]=simulate(m,40);rng(39);[b,z2,v2]=simulate(m,40);",
        );
        check(&mut e, "isequal(a,b) && isequal(z,z2) && isequal(v,v2)");
    }
    let mut e = Engine::default();
    // simulate Name-Value defaults: stationary mean, zero E0, unconditional V0.
    run(
        &mut e,
        "m=arima(Constant=1,AR=0.5,MA=0.2,Variance=garch(Constant=0.1,GARCH=0.6,ARCH=0.2));rng(4);[a,z,v]=simulate(m,20);rng(4);[b,z2,v2]=simulate(m,20,Y0=2,E0=0,V0=0.5);",
    );
    check(&mut e, "isequal(a,b) && isequal(z,z2) && isequal(v,v2)");
}

#[test]
fn variance_assignment_distribution_display_and_shared_value_semantics() {
    let mut e = Engine::default();
    run(
        &mut e,
        "v=garch(1,1);m=arima(1,0,1);m.Variance=v;w=m.Variance;other=m;v.Constant=0.2;m.Variance.Constant=0.3;",
    );
    check(
        &mut e,
        "isa(w,'garch') && isnan(w.Constant) && isnan(other.Variance.Constant) && m.Variance.Constant==0.3 && v.Constant==0.2",
    );
    let out = e.execute("m").unwrap();
    let text = format!("{out:?}");
    assert!(
        text.contains("ARIMA(1,0,1) Model with GARCH(1,1) Conditional Variance"),
        "{text}"
    );
    assert!(text.contains("[GARCH(1,1) Model]"), "{text}");
    assert!(text.contains("P: 1") && text.contains("Q: 1"), "{text}");
    for kind in ["garch", "egarch", "gjr"] {
        run(
            &mut e,
            &format!(
                "v={kind}(1,1);v.Distribution=struct('Name','t','DoF',9);m=arima('ARLags',1,'Variance',v);w=m.Variance;"
            ),
        );
        check(
            &mut e,
            "strcmp(w.Distribution.Name,'Gaussian') && v.Distribution.DoF==9",
        );
        run(
            &mut e,
            "a=m;m.Distribution=struct('Name','t','DoF',7);w=m.Variance;m.Distribution.DoF=8;",
        );
        check(
            &mut e,
            "w.Distribution.DoF==7 && m.Variance.Distribution.DoF==8 && strcmp(a.Variance.Distribution.Name,'Gaussian')",
        );
        run(&mut e, "m.Variance.Distribution='Gaussian';");
        check(&mut e, "m.Variance.Distribution.DoF==8");
        run(&mut e, "m.Distribution='Gaussian';m.Variance=2;");
        check(&mut e, "m.Variance==2");
        run(
            &mut e,
            &format!("m=arima('Distribution',struct('Name','t','DoF',5),'Variance',{kind}(1,1));"),
        );
        check(&mut e, "m.Variance.Distribution.DoF==5");
    }
    // The nested value remains reachable when its source variable is cleared
    // and the object table sweeps hundreds of temporary models.
    run(
        &mut e,
        "m=arima(Variance=garch(Constant=0.1,ARCH=0.2));clear v w other a;for k=1:600;tmp=garch(1,1);end;clear tmp;w=m.Variance;",
    );
    check(
        &mut e,
        "isa(w,'garch') && w.Constant==0.1 && w.ARCH{1}==0.2",
    );
}

#[test]
fn documented_presample_paths_alignment_and_validation() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=arima(Constant=0.1,AR=0.4,MA=0.2,Variance=gjr(Constant=0.1,GARCH=0.5,ARCH=0.1,Leverage=0.2));y=[1 2;3 4;2 1];[z,v,ll]=infer(m,y,Y0=0,E0=[0.2 -0.3],V0=[1 2]);[za,va,la]=infer(m,y(:,1),Y0=0,E0=0.2,V0=1);[zb,vb,lb]=infer(m,y(:,2),Y0=0,E0=-0.3,V0=2);",
    );
    check(
        &mut e,
        "isequal(z,[za zb]) && isequal(v,[va vb]) && isequal(ll,[la lb])",
    );
    run(
        &mut e,
        "[f,s,v]=forecast(m,4,[1;2],E0=[0.2 -0.3],V0=[1 2]);[fa,sa,va]=forecast(m,4,[1;2],E0=0.2,V0=1);[fb,sb,vb]=forecast(m,4,[1;2],E0=-0.3,V0=2);",
    );
    check(
        &mut e,
        "isequal(f,[fa fb]) && isequal(s,[sa sb]) && isequal(v,[va vb])",
    );
    run(
        &mut e,
        "[z,v,ll]=infer(m,[1;2;3],Y0=[8;NaN;0.4],E0=[NaN;0.2],V0=[4;5;1]);[za,va,la]=infer(m,[1;2;3],Y0=0.4,E0=0.2,V0=1);",
    );
    check(&mut e, "isequal(z,za) && isequal(v,va) && ll==la");
    // Long Y0 uses only its tail; V0 width can determine the path count.
    run(&mut e, "[f,s,v]=forecast(m,3,[9;1],E0=0.2,V0=[1 2 3]);");
    assert_eq!(values(&e, "f").len(), 9);
    for (source, msg) in [
        ("forecast(m,2,[1 2],E0=[1 2 3])", "one column"),
        ("infer(m,[1;2],V0=0)", "positive"),
        ("simulate(m,2,V0=-1)", "positive"),
        ("forecast(m,2,1,V0=Inf)", "finite"),
        ("infer(m,[1;2],E0=Inf)", "finite"),
        ("forecast(m,2)", "Y0"),
        ("forecast(m,2,1,Y0=1)", "only once"),
        ("infer(m,[1;2],uifigure=1)", "Unknown option"),
        ("simulate(m,2,NumPaths=2000000)", "2000000"),
        ("infer(arima(Variance=garch(1,1)),[1;2])", "fully specified"),
        ("simulate(m,2,NumPaths=0)", "integer"),
    ] {
        fail(&mut e, source, msg);
    }
    run(
        &mut e,
        "m.Variance=egarch(Constant=-0.1,GARCH=0.5,ARCH={0.1 0.2},Leverage=0.1);",
    );
    fail(&mut e, "infer(m,[1;2],E0=1,V0=[1;1])", "E0");
    fail(&mut e, "simulate(m,2,E0=[0;0],V0=1)", "V0");
    for kind in ["garch", "egarch", "gjr"] {
        for offset in ["1", "NaN"] {
            fail(
                &mut e,
                &format!("arima('Variance',{kind}(1,1,'Offset',{offset}))"),
                "Offset must be 0",
            );
            run(&mut e, &format!("v={kind}(1,1,'Offset',{offset});"));
            fail(&mut e, "m.Variance=v", "Offset must be 0");
        }
    }
    fail(&mut e, "m.Variance.Offset=1", "Offset must be 0");
    fail(
        &mut e,
        "m.Variance=struct('Name','garch')",
        "Variance must be",
    );
    fail(&mut e, "summarize(m,[1;2;3])", "Too many input");
    fail(&mut e, "estimate(m)", "Not enough input");
}

#[test]
fn composite_regression_forecasts_and_simulation_round_trip() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=arima(Constant=0.1,AR=0.4,MA=0.2,Beta=0.3,Variance=garch(Constant=0.1,GARCH=0.6,ARCH=0.2));x=(1:20)';rng(45);[y,z,v]=simulate(m,20,X=x,Y0=0.2,E0=0.1,V0=1);[zi,vi]=infer(m,y,X=x,Y0=0.2,E0=0.1,V0=1);",
    );
    near(&values(&e, "zi"), &values(&e, "z"));
    near(&values(&e, "vi"), &values(&e, "v"));
    run(
        &mut e,
        "[a,am,av]=forecast(m,5,y,X0=x(2:end,:),XF=ones(5,1));[b,bm,bv]=forecast(m,5,y,E0=zi(2:end),XF=ones(5,1));",
    );
    // Both histories begin at Y(1), with zero initialization before Y(2).
    // Supply the explicitly inferred history to check X0 alignment exactly.
    run(
        &mut e,
        "[e0,v0]=infer(m,y(2:end),Y0=y(1),X=x(2:end,:));[b,bm,bv]=forecast(m,5,y,E0=e0,XF=ones(5,1));",
    );
    near(&values(&e, "a"), &values(&e, "b"));
    near(&values(&e, "am"), &values(&e, "bm"));
    near(&values(&e, "av"), &values(&e, "bv"));
    // X0 is ignored when E0 is supplied, including an invalid type.
    run(
        &mut e,
        "[c,cm,cv]=forecast(m,5,y,E0=e0,X0='ignored',XF=ones(5,1));",
    );
    check(&mut e, "isequal(c,b) && isequal(cm,bm) && isequal(cv,bv)");
}

#[test]
fn composite_backcast_limits_and_cancellation() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let mut e = Engine::with_limits(matrust::engine::Limits::local());
    // arima.infer Y0 defaults to backward prediction. For this AR(1),
    // y_pre = 2 + 0.5*(4-2) = 3; the residual second moment is 8.5/3.
    run(
        &mut e,
        "m=arima(Constant=1,AR=0.5,Variance=garch(Constant=0.1,GARCH=0.6,ARCH=0.2));[z,v]=infer(m,[4;1;3]);",
    );
    near(&values(&e, "z"), &[1.5, -2., 1.5]);
    near(&values(&e, "v"), &[1.8, 1.63, 1.878]);
    fail(&mut e, "infer(m,[1;2],Y0=NaN)", "Y0");
    fail(&mut e, "infer(m,[1;2],E0=NaN)", "E0");
    let cancellation = e.cancellation();
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let handle = std::thread::spawn(move || {
        while !finished.load(Ordering::Relaxed) {
            cancellation.store(true, Ordering::Relaxed);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });
    let result = e.execute("simulate(m,2000000)");
    done.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    assert!(result.unwrap_err().to_lowercase().contains("cancel"));
    let mut limited = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 16 * 1024,
        ..matrust::engine::Limits::browser()
    });
    fail(
        &mut limited,
        "simulate(arima(Constant=0,Variance=garch(Constant=1)),1000)",
        "memory limit",
    );
}

#[test]
#[ignore = "release benchmark; run explicitly with --ignored --nocapture"]
fn composite_fixed_paths_release_benchmark() {
    for workers in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                let mut e = Engine::with_limits(matrust::engine::Limits::local());
                for kind in ["garch", "gjr", "egarch"] {
                    let variance = if kind == "egarch" {
                        "egarch(Constant=-0.1,GARCH=0.7,ARCH=0.2,Leverage=-0.1)".to_owned()
                    } else {
                        format!("{kind}(Constant=0.1,GARCH=0.6,ARCH=0.2)")
                    };
                    run(
                        &mut e,
                        &format!(
                            "m=arima(Constant=0.1,AR=0.5,MA=0.2,Variance={variance});rng(31);"
                        ),
                    );
                    for (method, source) in [
                        ("simulate", "[y,z,v]=simulate(m,100000,Y0=0,E0=0,V0=1);"),
                        ("infer", "[z,v,ll]=infer(m,y,Y0=0,E0=0,V0=1);"),
                        ("forecast", "[f,s,vf]=forecast(m,100000,y,E0=z,V0=v);"),
                    ] {
                        let start = std::time::Instant::now();
                        run(&mut e, source);
                        eprintln!(
                            "{kind} {method}, n=100000, workers={workers}, elapsed={:?}",
                            start.elapsed()
                        );
                    }
                }
            });
    }
}

fn fit_sample(e: &mut Engine, c: &Json) {
    for key in ["y", "y0", "e0", "v0"] {
        insert(e, key, nums(&c[key]), 1);
    }
    run(
        e,
        if c["kind"] == "garch" {
            "m=arima('ARLags',1,'Variance',garch(1,1));"
        } else {
            "m=arima('ARLags',1,'MALags',1,'Variance',gjr(1,1),'Distribution','t');"
        },
    );
}

#[test]
fn joint_estimate_seeded_n2000_against_scipy_and_arch() {
    for c in refs()["fits"].as_array().unwrap() {
        let mut e = Engine::default();
        fit_sample(&mut e, c);
        let out = e.execute("[f,cov,ll,info]=estimate(m,y,Y0=y0,E0=e0,V0=v0,Display='params');params=info.X;s=summarize(f);mt=s.Table;vt=s.VarianceTable;").unwrap();
        let text = format!("{out:?}");
        for label in [
            "Effective Sample Size: 2000",
            "StandardError",
            "TStatistic",
            "PValue",
            "Conditional Variance Model",
            "GARCH{1}",
            "ARCH{1}",
        ] {
            assert!(text.contains(label), "{label}: {text}");
        }
        let want = nums(&c["parameters"]);
        let got = values(&e, "params");
        for (i, (&a, &b)) in got.iter().zip(&want).enumerate() {
            assert!(
                (a - b).abs() < 1e-4,
                "{} parameter {i}: {a} != {b}",
                c["kind"]
            );
        }
        assert!((values(&e, "ll")[0] - c["logl"].as_f64().unwrap()).abs() < 1e-6);
        let covariance = values(&e, "cov");
        assert_eq!(covariance.len(), want.len() * want.len());
        for (i, (&a, b)) in covariance.iter().zip(nums(&c["covariance"])).enumerate() {
            assert!(
                (a - b).abs() < 2e-5 * (1. + b.abs()),
                "covariance {i}: {a} != {b}"
            );
        }
        // arch drops the first AR row and backcasts variance; it is a loose
        // cross-check, while the SciPy oracle uses the same explicit presamples.
        if let Some(arch) = c.get("arch_parameters") {
            for (&a, b) in got.iter().zip(nums(arch)) {
                assert!((a - b).abs() < 0.05);
            }
        }
        let (mean_rows, var_rows) = if c["kind"] == "garch" { (2, 3) } else { (4, 4) };
        match (e.array("mt").unwrap(), e.array("vt").unwrap()) {
            (Array::Table(mt), Array::Table(vt)) => {
                assert_eq!(mt.height(), mean_rows);
                assert_eq!(vt.height(), var_rows);
                assert_eq!(
                    vt.row_names().unwrap()[..3],
                    ["Constant", "GARCH{1}", "ARCH{1}"]
                );
                if var_rows == 4 {
                    assert_eq!(vt.row_names().unwrap()[3], "Leverage{1}");
                }
            }
            x => panic!("{x:?}"),
        }
        check(
            &mut e,
            "s.SampleSize==2000 && s.NumEstimatedParameters==numel(params) && all(diag(cov)>0) && max(max(abs(cov-cov')))<1e-12 && isnan(m.Constant) && isnan(m.Variance.Constant)",
        );
        run(
            &mut e,
            "[z,v,li]=infer(f,y,Y0=y0,E0=e0,V0=v0);vparams=[f.Variance.Constant;f.Variance.GARCH{1};f.Variance.ARCH{1}];",
        );
        assert!((values(&e, "li")[0] - values(&e, "ll")[0]).abs() < 1e-9);
        run(
            &mut e,
            "mv=mt.Value;vv=vt.Value;ms=mt.StandardError;vs=vt.StandardError;",
        );
        let mean_ids = if c["kind"] == "garch" {
            vec![0, 1]
        } else {
            vec![0, 1, 2, 7]
        };
        let var_ids = if c["kind"] == "garch" {
            vec![2, 3, 4]
        } else {
            vec![3, 4, 5, 6]
        };
        for (value, se, ids) in [("mv", "ms", mean_ids), ("vv", "vs", var_ids)] {
            near(
                &values(&e, value),
                &ids.iter().map(|&i| got[i]).collect::<Vec<_>>(),
            );
            near(
                &values(&e, se),
                &ids.iter()
                    .map(|&i| covariance[i * got.len() + i].sqrt())
                    .collect::<Vec<_>>(),
            );
        }
        let start = if c["kind"] == "garch" { 2 } else { 3 };
        near(&values(&e, "vparams"), &got[start..start + 3]);
        let out = e.execute("summarize(f);").unwrap();
        let sum_text = out.output.join("\n");
        assert!(
            sum_text.contains("Conditional Variance Model"),
            "{sum_text}"
        );
        let ll_val: f64 = sum_text
            .lines()
            .find(|l| l.contains("LogLikelihood:"))
            .and_then(|l| l.split(':').nth(1))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!((ll_val - values(&e, "ll")[0]).abs() < 1.0, "{sum_text}");
    }
}

#[test]
fn composite_estimate_presample_defaults_and_listwise_deletion() {
    for c in refs()["cases"].as_array().unwrap() {
        let mut e = Engine::default();
        sample(&mut e, c);
        // Fixed-parameter estimate still evaluates the joint likelihood, using
        // the same backcast/zero E0/residual-second-moment V0 rules as infer.
        for pre in ["Y0=y0,E0=e0,V0=v0", "Y0=y0,E0=e0", "Y0=y0", ""] {
            let comma = if pre.is_empty() { "" } else { "," };
            run(
                &mut e,
                &format!(
                    "[f,cov,ll]=estimate(m,y,{pre}{comma}Display='off');[z,v,li]=infer(m,y{comma}{pre});s=summarize(f);"
                ),
            );
            assert!((values(&e, "ll")[0] - values(&e, "li")[0]).abs() < 1e-10);
            check(
                &mut e,
                "s.NumEstimatedParameters==0 && all(all(cov==0)) && s.SampleSize==numel(y)",
            );
        }
    }
    let mut e = Engine::default();
    run(
        &mut e,
        "m=arima(Constant=0.1,AR=0.4,MA=0.2,Variance=gjr(Constant=0.1,GARCH=0.5,ARCH=0.1,Leverage=0.2));[a,ca,la]=estimate(m,[1;2;NaN;3],Y0=[8;NaN;0.4],E0=[NaN;0.2],V0=[4;5;1],Display='off');[b,cb,lb]=estimate(m,[1;2;3],Y0=0.4,E0=0.2,V0=1,Display='off');",
    );
    check(&mut e, "la==lb && isequal(ca,cb)");
    for (source, message) in [
        ("estimate(m,[1;2;3],V0=0)", "positive"),
        ("estimate(m,[1;2;3],V0=Inf)", "finite"),
        ("estimate(m,[1;2;3],E0=NaN)", "E0"),
        ("estimate(m,[1;2;3],Y0=[0 0])", "column vectors"),
        ("estimate(m,[1;2;3],V0=[1 1])", "column vectors"),
        ("estimate(m,[1;2;3],Variance0=1)", "cell vector"),
        (
            "estimate(m,[1;2;3],Variance0={'uifigure',1})",
            "Unknown option",
        ),
    ] {
        fail(&mut e, source, message);
    }
    run(
        &mut e,
        "m.Variance=egarch(Constant=-0.1,GARCH=0.5,ARCH={0.1 0.2},Leverage=0.1);",
    );
    fail(&mut e, "estimate(m,[1;2;3],E0=1,V0=[1;1])", "E0");
    fail(&mut e, "estimate(m,[1;2;3],E0=[0;0],V0=1)", "V0");
}

#[test]
fn composite_joint_regression_matches_scipy_ar_mean_fixture() {
    let c = &refs()["fits"][0];
    let mut e = Engine::default();
    fit_sample(&mut e, c);
    // Explicit lagged responses as an exogenous predictor have exactly the
    // AR(1) fixture's likelihood; this checks Beta's slot before variance.
    run(
        &mut e,
        "x=[y0;y(1:end-1)];m=arima('Beta',NaN,'Variance',garch(1,1));[f,cov,ll,info]=estimate(m,y,X=x,E0=e0,V0=v0,Display='off');params=info.X;s=summarize(f);",
    );
    for (&a, b) in values(&e, "params").iter().zip(nums(&c["parameters"])) {
        assert!((a - b).abs() < 1e-4, "{a} != {b}");
    }
    assert!((values(&e, "ll")[0] - c["logl"].as_f64().unwrap()).abs() < 1e-6);
    check(
        &mut e,
        "s.SampleSize==2000 && s.NumEstimatedParameters==5 && height(s.Table)==2 && height(s.VarianceTable)==3 && size(cov,1)==5",
    );
}

#[test]
fn composite_starts_constraints_and_fixed_covariance_order() {
    let c = &refs()["fits"][1];
    let mut e = Engine::default();
    fit_sample(&mut e, c);
    run(
        &mut e,
        "[f,cov,ll,info]=estimate(m,y,Y0=y0,E0=e0,V0=v0,Constant0=0.1,AR0=0.3,MA0=-0.2,DoF0=8,Variance0={'Constant0',0.1,'GARCH0',0.6,'ARCH0',0.1,'Leverage0',0.2},Options=optimset('MaxIter',0),Display='off');initial=info.X0;final=info.X;",
    );
    near(
        &values(&e, "initial"),
        &[0.1, 0.3, -0.2, 0.1, 0.6, 0.1, 0.2, 8.],
    );
    near(&values(&e, "initial"), &values(&e, "final"));
    check(
        &mut e,
        "info.exitflag==0 && f.Distribution.DoF==8 && f.Variance.Distribution.DoF==8",
    );
    for (args, msg) in [
        ("AR0=1.1", "stationary"),
        ("MA0=1.1", "invertible"),
        ("DoF0=2", "exceed 2"),
        ("Variance0={'Constant0',-1}", "positive"),
        ("Variance0={'GARCH0',0.95,'ARCH0',0.2}", "stationarity"),
        ("Variance0={'ARCH0',0.01,'Leverage0',-0.2}", "nonnegative"),
        ("Variance0={'ARCH0',[0.1 0.2]}", "coefficient count"),
        ("Variance0={'Offset0',1}", "Unknown option"),
    ] {
        fail(&mut e, &format!("estimate(m,y,{args},Display='off')"), msg);
    }
    run(
        &mut e,
        "m.Constant=0.13;m.Variance.GARCH=0.66;m.Distribution=struct('Name','t','DoF',7.5);[f,cov,ll,info]=estimate(m,y,Y0=y0,E0=e0,V0=v0,Display='off');s=summarize(f);",
    );
    check(
        &mut e,
        "s.NumEstimatedParameters==5 && all(cov(1,:)==0) && all(cov(:,1)==0) && all(cov(5,:)==0) && all(cov(:,5)==0) && all(cov(8,:)==0) && all(cov(:,8)==0) && f.Constant==0.13 && f.Variance.GARCH{1}==0.66 && f.Distribution.DoF==7.5",
    );
    run(
        &mut e,
        "m=arima(Variance=egarch(2,2));[f,cov,ll,info]=estimate(m,y,Options=optimset('MaxIter',0),Display='off');s=summarize(f);initial=info.X0;",
    );
    // Zero-valued higher-lag defaults retain all template parameter slots.
    assert_eq!(values(&e, "initial").len(), 8);
    check(
        &mut e,
        "s.NumEstimatedParameters==8 && height(s.VarianceTable)==7 && numel(cov)==64",
    );
    run(&mut e, "a=summarize(m);");
    check(&mut e, "isa(a,'arima') && isnan(a.Constant)");
    run(
        &mut e,
        "m=arima(Constant=0,Variance=garch(Constant=1,ARCH=NaN));[f,cov,ll]=estimate(m,y,Variance0={'ARCH0',0},Options=optimset('MaxIter',0),Display='off');s=summarize(f);",
    );
    check(
        &mut e,
        "s.NumEstimatedParameters==1 && size(cov,1)==3 && cov(3,3)>0 && isfinite(cov(3,3)) && height(s.VarianceTable)==2",
    );
}

#[test]
#[ignore = "release benchmark; run explicitly with --ignored --nocapture"]
fn composite_joint_estimate_release_benchmark() {
    for workers in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                for c in refs()["fits"].as_array().unwrap() {
                    let mut e = Engine::with_limits(matrust::engine::Limits::local());
                    fit_sample(&mut e, c);
                    let start = std::time::Instant::now();
                    run(
                        &mut e,
                        "[f,cov,ll]=estimate(m,y,Y0=y0,E0=e0,V0=v0,Display='off');",
                    );
                    eprintln!(
                        "{} joint estimate n=2000 workers={workers}: {:?}",
                        c["kind"],
                        start.elapsed()
                    );
                }
            });
    }
}

#[test]
fn composite_display_modes_and_estimation_cancellation() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=arima(Constant=0.1,AR=0.4,Variance=garch(Constant=0.1,GARCH=0.6,ARCH=0.2));y=[1;0;2;1;0;1];",
    );
    for display in ["params", "full", "iter", "diagnostics", "off"] {
        let out = e
            .execute(&format!("f=estimate(m,y,Y0=0,Display='{display}');"))
            .unwrap();
        let text = format!("{out:?}");
        assert_eq!(
            text.contains("GARCH{1}"),
            matches!(display, "params" | "full"),
            "{text}"
        );
        assert_eq!(
            text.contains("exit flag"),
            matches!(display, "diagnostics" | "full"),
            "{text}"
        );
    }
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let cancel = e.cancellation();
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let handle = std::thread::spawn(move || {
        while !finished.load(Ordering::Relaxed) {
            cancel.store(true, Ordering::Relaxed);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });
    let result = e
        .execute("estimate(arima('ARLags',1,'Variance',garch(1,1)),ones(200000,1),Display='off');");
    done.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    assert!(result.unwrap_err().to_lowercase().contains("cancel"));
    let mut limited = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 16 * 1024,
        ..matrust::engine::Limits::browser()
    });
    fail(
        &mut limited,
        "estimate(arima('ARLags',1,'Variance',garch(1,1)),ones(1000,1),Display='off')",
        "memory limit",
    );
    let mut e2 = Engine::default();
    run(
        &mut e2,
        "m=arima(Constant=0,AR=0.5,MA=0.2,Beta=1,Variance=garch(Constant=0.1,GARCH=0.6,ARCH=0.2));f=forecast(m,1,[1;2;3;4],X0=[0;0;NaN],XF=0);",
    );
    assert!((values(&e2, "f")[0] - 1.84).abs() < 1e-14);
}
