//! Expected values and seeded n=2000 fixtures: tool/refs/econ/garch.py (NumPy/SciPy).
//! arch cross-checks fixed recursions with matching explicit presamples; its default
//! backcast differs, and arch p=MATLAB Q, q=MATLAB P. No MATLAB was used.
use matrust::{Array, Engine, Matrix};
use std::sync::atomic::AtomicBool;
#[allow(clippy::excessive_precision)]
mod reference {
    include!("fixtures/econ/garch.rs");
}
use reference::*;
fn run(e: &mut Engine, s: &str) {
    e.execute(s).unwrap_or_else(|err| panic!("{s}: {err}"));
}
fn check(e: &mut Engine, s: &str) {
    run(e, &format!("assert({s});"));
}
fn vals(e: &Engine, n: &str) -> Vec<f64> {
    match e.array(n).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        a => panic!("{a:?}"),
    }
}
fn near(e: &Engine, n: &str, want: &[f64], tol: f64) {
    let got = vals(e, n);
    assert_eq!(got.len(), want.len(), "{n}");
    for (i, (a, b)) in got.iter().zip(want).enumerate() {
        assert!((a - b).abs() <= tol, "{n}[{i}] {a:.17e} != {b:.17e}");
    }
}
fn fails(e: &mut Engine, s: &str, part: &str) {
    let err = e.execute(s).unwrap_err();
    assert!(err.contains(part), "{s}: {err}");
}
fn sample(e: &mut Engine, data: &str) {
    let y = data
        .lines()
        .map(|l| l.parse::<f64>().unwrap())
        .collect::<Vec<_>>();
    e.insert_array(
        "y",
        Array::Numeric(Matrix::from_column_vec(y.len(), 1, y).unwrap()),
    )
    .unwrap();
}
#[test]
fn constructors_display_properties_and_value_assignment() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=garch;n=egarch;j=gjr;assert(m.P==0 && n.Q==0 && j.Q==0);",
    );
    for name in ["garch", "egarch", "gjr"] {
        let output = e
            .execute(&format!("m={name}(1,2)"))
            .unwrap()
            .output
            .join("\n");
        assert!(
            output.contains(&format!(
                "{}(1,2) Conditional Variance Model",
                name.to_uppercase()
            )),
            "{output}"
        );
        assert!(output.contains("at lags [1 2]"), "{output}");
        assert!(output.contains("Offset: 0"), "{output}");
        check(
            &mut e,
            &format!(
                "isa(m,'{name}') && strcmp(class(m),'{name}') && m.P==1 && m.Q==2 && isnan(m.Constant) && m.Offset==0"
            ),
        );
        run(
            &mut e,
            "copy=m;m.Constant=0.1;m.Description=\"test model\";m.SeriesName='Return';m.Distribution='t';m.Distribution.DoF=8;",
        );
        check(
            &mut e,
            "isnan(copy.Constant) && m.Constant==0.1 && strcmp(m.Description,'test model') && strcmp(m.SeriesName,'Return') && m.Distribution.DoF==8",
        );
        fails(&mut e, "m.Distribution.DoF=2;", "DoF must exceed 2");
        fails(&mut e, "m.P=3;", "read-only");
        fails(&mut e, "m.Q=3;", "read-only");
        fails(&mut e, &format!("{name}(-1,1)"), "nonnegative");
        fails(&mut e, &format!("{name}(1,0)"), "positive ARCH");
        fails(&mut e, &format!("{name}(1.5,1)"), "integer");
        fails(&mut e, &format!("{name}(129,1)"), "128");
        fails(
            &mut e,
            &format!("{name}('GARCHLags',[1 1],'ARCHLags',1)"),
            "unique positive",
        );
        fails(
            &mut e,
            &format!("{name}('ARCHLags',[1 3],'ARCH',{{0.1}})"),
            "equal lengths",
        );
        fails(
            &mut e,
            &format!("{name}('Distribution',struct('Name','t'))"),
            "requires DoF",
        );
        fails(
            &mut e,
            &format!("{name}('Distribution','laplace')"),
            "Gaussian or t",
        );
        fails(&mut e, &format!("{name}('Bogus',1)"), "Unrecognized");
        fails(&mut e, "infer(m,[1;2])", "fully specified");
    }
    run(
        &mut e,
        "m=garch('Constant',0.1,'GARCHLags',[1 3],'GARCH',{0.3 0.2},'ARCHLags',[2 4],'ARCH',{0.1 0.2});",
    );
    check(
        &mut e,
        "m.P==3 && m.Q==4 && isequal(m.GARCHLags,[1 3]) && isequal(m.ARCHLags,[2 4]) && isequal(m.GARCH,{0.3 0 0.2})",
    );
    run(&mut e, "n=garch(3,2);n.GARCH{2}=0;");
    check(&mut e, "isequal(n.GARCHLags,[1 3])");
    run(&mut e, "n.GARCH{2}=0.2;");
    check(
        &mut e,
        "n.P==3 && isequal(n.GARCHLags,[1 2 3]) && n.GARCH{2}==0.2",
    );
    fails(&mut e, "garch('Constant',0)", "positive scalar");
    fails(&mut e, "gjr('Constant',-1)", "positive scalar");
    fails(&mut e, "garch('GARCH',0.9,'ARCH',0.2)", "stationarity");
    fails(&mut e, "garch('ARCH',-0.2)", "nonnegative");
    fails(
        &mut e,
        "gjr('ARCH',0.1,'Leverage',-0.2)",
        "ARCH plus Leverage",
    );
    fails(
        &mut e,
        "gjr('GARCH',0.8,'ARCH',0.1,'Leverage',0.3)",
        "stationarity",
    );
    fails(&mut e, "egarch('GARCH',{1.1},'ARCH',0.2)", "unit circle");
    run(
        &mut e,
        "m=egarch('Constant',-0.2,'GARCH',{1.2 -0.4},'ARCH',-0.1,'Leverage',0.2);[v,l]=infer(m,[1;2;-1]);",
    );
    check(&mut e, "all(v>0) && m.P==2");
    run(&mut e, "v2=m.infer([1;2;-1]);pn=properties(m);");
    check(&mut e, "norm(v-v2)==0 && any(strcmp(pn,'Leverage'))");
    fails(&mut e, "m.GARCH={1.5 -0.2};", "unit circle");
    // Shared method names must leave unknown first arguments to normal dispatch.
    for method in ["estimate", "forecast", "infer", "simulate", "summarize"] {
        fails(&mut e, &format!("{method}(1)"), "Undefined function");
    }
}
#[test]
fn fixed_infer_gaussian_t_higher_order_asymmetric_offset_and_defaults() {
    let mut e = Engine::default();
    type Case = (&'static str, &'static [f64], f64, &'static [f64], f64);
    let cases: [Case; 6] = [
        (
            "garch('Constant',0.1,'GARCH',0.7,'ARCH',0.2,'Offset',0.3)",
            G11_V,
            G11_LOGL,
            G11_DEFAULT_V,
            G11_DEFAULT_LOGL,
        ),
        (
            "garch('Constant',0.1,'GARCH',0.7,'ARCH',0.2,'Offset',0.3,'Distribution',struct('Name','t','DoF',7.5))",
            G11_T_V,
            G11_T_LOGL,
            G11_T_DEFAULT_V,
            G11_T_DEFAULT_LOGL,
        ),
        (
            "garch('Constant',0.12,'GARCH',{0.5 0.2},'ARCH',0.15,'Offset',0.3)",
            G21_V,
            G21_LOGL,
            G21_DEFAULT_V,
            G21_DEFAULT_LOGL,
        ),
        (
            "egarch('Constant',-0.07,'GARCH',0.85,'ARCH',0.2,'Leverage',-0.15,'Offset',0.3)",
            EG11_V,
            EG11_LOGL,
            EG11_DEFAULT_V,
            EG11_DEFAULT_LOGL,
        ),
        (
            "gjr('Constant',0.08,'GARCH',0.7,'ARCH',0.1,'Leverage',0.2,'Offset',0.3)",
            GJR11_V,
            GJR11_LOGL,
            GJR11_DEFAULT_V,
            GJR11_DEFAULT_LOGL,
        ),
        (
            "egarch('Constant',-0.07,'GARCH',0.85,'ARCH',0.2,'Leverage',-0.15,'Offset',0.3,'Distribution',struct('Name','t','DoF',6.5))",
            EG11_T_V,
            EG11_T_LOGL,
            EG11_T_DEFAULT_V,
            EG11_T_DEFAULT_LOGL,
        ),
    ];
    for (constructor, v, ll, vd, lld) in cases {
        run(
            &mut e,
            &format!(
                "m={constructor};y=[0.8;-0.4;1.2;-1.5;0.2;0.9;-0.8;0.4]+0.3;[v,l]=infer(m,y,'E0',[-0.3;0.5],'V0',[0.8;1.1]);[vd,ld]=infer(m,y);"
            ),
        );
        near(&e, "v", v, 1e-10);
        near(&e, "l", &[ll], 1e-10);
        near(&e, "vd", vd, 1e-10);
        near(&e, "ld", &[lld], 1e-10);
        run(&mut e, "[v2,l2]=infer(m,[y y],E0=[-0.3;0.5],V0=[0.8;1.1]);");
        check(
            &mut e,
            "isequal(size(v2),[8 2]) && norm(v2(:,1)-v)==0 && norm(v2(:,2)-v)==0 && isequal(size(l2),[1 2])",
        );
        near(&e, "l2", &[ll, ll], 1e-10);
    }
}
#[test]
fn forecast_closed_form_and_documented_egarch_log_method() {
    let mut e = Engine::default();
    for (constructor, want) in [
        ("garch(Constant=0.1,GARCH=0.7,ARCH=0.2,Offset=0.3)", G11_F),
        (
            "garch(Constant=0.1,GARCH=0.7,ARCH=0.2,Offset=0.3,Distribution=struct('Name','t','DoF',7.5))",
            G11_T_F,
        ),
        (
            "garch(Constant=0.12,GARCH={0.5 0.2},ARCH=0.15,Offset=0.3)",
            G21_F,
        ),
        (
            "egarch(Constant=-0.07,GARCH=0.85,ARCH=0.2,Leverage=-0.15,Offset=0.3)",
            EG11_F,
        ),
        (
            "gjr(Constant=0.08,GARCH=0.7,ARCH=0.1,Leverage=0.2,Offset=0.3)",
            GJR11_F,
        ),
        (
            "egarch(Constant=-0.07,GARCH=0.85,ARCH=0.2,Leverage=-0.15,Offset=0.3,Distribution=struct('Name','t','DoF',6.5))",
            EG11_T_F,
        ),
    ] {
        run(
            &mut e,
            &format!(
                "m={constructor};f=forecast(m,8,[-0.3;0.5]+0.3,'V0',[0.8;1.1]);f2=forecast(m,8,'Y0',[-0.3;0.5]+0.3,'V0',[0.8;1.1]);f3=forecast(m,8,[-0.3;0.5]+0.3,'V0',[0.8 0.8;1.1 1.1]);"
            ),
        );
        near(&e, "f", want, 1e-10);
        check(
            &mut e,
            "isequal(f,f2) && norm(f3(:,1)-f)==0 && norm(f3(:,2)-f)==0",
        );
        run(
            &mut e,
            "y=[1;-1;0.4;0.8;-0.6;1.1]+0.3;v=infer(m,y);f=forecast(m,5,y);f2=forecast(m,5,y,'V0',v);",
        );
        check(&mut e, "norm(f-f2)<1e-12");
    }
    run(
        &mut e,
        "m=garch(Constant=0.1,GARCH=0.7,ARCH=0.2);f=forecast(m,8,[0.5],'V0',1.1);closed=1+(f(1)-1)*0.9.^(0:7)';",
    );
    check(&mut e, "norm(f-closed)<1e-12");
    run(
        &mut e,
        "m=gjr(Constant=0.08,GARCH=0.7,ARCH=0.1,Leverage=0.2);f=forecast(m,8,-0.5,'V0',1.1);closed=0.8+(f(1)-0.8)*0.9.^(0:7)';",
    );
    check(&mut e, "norm(f-closed)<1e-12");
    // garch.forecast: fewer than max(P,Q)+P responses start from the unconditional
    // variance (1 here), so 0.1 + 0.7*1 + 0.2*0.5^2.
    run(
        &mut e,
        "f=forecast(garch(Constant=0.1,GARCH=0.7,ARCH=0.2),1,0.5);",
    );
    check(&mut e, "abs(f-0.85)<1e-12");
}
#[test]
fn presample_errors_missing_rows_paths_and_output_limits() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=egarch(Constant=-0.1,GARCH=0.7,ARCH={0.1 0.1},Leverage=-0.1);",
    );
    fails(&mut e, "infer(m,[1;2],'E0',1)", "Insufficient presample");
    fails(&mut e, "infer(m,[1;2],'V0',1)", "Insufficient presample");
    fails(&mut e, "simulate(m,3,'V0',[1;0])", "positive");
    fails(&mut e, "forecast(m,3,1)", "Insufficient presample");
    run(
        &mut e,
        "m=garch(Constant=0.1,GARCH=0.7,ARCH=0.2);[v,l]=infer(m,[1 NaN;2 1;3 2]);[v2,l2]=infer(m,[2 1;3 2]);",
    );
    check(&mut e, "isequal(v,v2) && isequal(l,l2)");
    run(
        &mut e,
        "v=infer(m,[1;2],E0=[99;NaN;0.4],V0=[99;3;1]);v2=infer(m,[1;2],E0=0.4,V0=1);",
    );
    check(&mut e, "isequal(v,v2)");
    fails(&mut e, "infer(m,[1;Inf])", "finite");
    fails(&mut e, "infer(m,[NaN;NaN])", "nonempty");
    fails(
        &mut e,
        "infer(m,ones(5,3),E0=ones(1,2))",
        "presample rows or paths",
    );
    fails(
        &mut e,
        "forecast(m,2,ones(2,3),V0=ones(2,2))",
        "same number of paths",
    );
    fails(&mut e, "simulate(m,10000000,NumPaths=2)", "10000000");
    fails(&mut e, "forecast(m,0,[1;2])", "positive integer");
    fails(&mut e, "[a,b,c]=infer(m,[1;2])", "Too many output");
    fails(&mut e, "[a,b]=garch(1,1)", "Too many output");
    fails(&mut e, "infer(m,[1;2],Bogus=1)", "Unrecognized");
    let mut limited = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 1024,
        ..matrust::engine::Limits::browser()
    });
    fails(
        &mut limited,
        "simulate(garch(Constant=1),1000)",
        "memory limit",
    );
}
#[test]
fn session_random_stream_reproducibility_shapes_and_unconditional_variance() {
    let mut e = Engine::default();
    for constructor in [
        "garch(Constant=0.1,GARCH=0.7,ARCH=0.2,Offset=0.3)",
        "egarch(Constant=-0.1,GARCH=0.8,ARCH=0.15,Leverage=-0.1)",
        "gjr(Constant=0.1,GARCH=0.7,ARCH=0.1,Leverage=0.2)",
        "garch(Constant=0.1,GARCH=0.7,ARCH=0.2,Distribution=struct('Name','t','DoF',8))",
    ] {
        run(
            &mut e,
            &format!(
                "m={constructor};rng(17);[v,y]=simulate(m,40,NumPaths=3);after=randn(1);rng(17);[v2,y2]=simulate(m,40,NumPaths=3);after2=randn(1);"
            ),
        );
        check(
            &mut e,
            "isequal(size(v),[40 3]) && isequal(v,v2) && isequal(y,y2) && after==after2 && all(v(:)>0)",
        );
        run(
            &mut e,
            "rng(2);[v,y]=simulate(m,40,E0=0.4,V0=1.1);v2=infer(m,y,E0=0.4,V0=1.1);",
        );
        check(&mut e, "norm(v-v2)<1e-12");
    }
    // No-lag Gaussian simulation consumes exactly the same randn stream.
    run(
        &mut e,
        "rng(4);[v,y]=simulate(garch(Constant=2,Offset=0.3),8);rng(4);want=sqrt(2)*randn(8,1)+0.3;",
    );
    check(&mut e, "norm(y-want)==0");
    for constructor in [
        "garch(Constant=0.1,GARCH=0.7,ARCH=0.2)",
        "gjr(Constant=0.1,GARCH=0.7,ARCH=0.1,Leverage=0.2)",
        "garch(Constant=0.1,GARCH=0.7,ARCH=0.2,Distribution=struct('Name','t','DoF',8))",
    ] {
        run(
            &mut e,
            &format!("rng(2718);[v,y]=simulate({constructor},200000);mv=mean(v);vy=mean(y.^2);"),
        );
        near(&e, "mv", &[1.], 0.06);
        near(&e, "vy", &[1.], 0.07);
    }
    run(
        &mut e,
        "rng(53);[v,y]=simulate(egarch(Constant=-0.1,GARCH=0.8,ARCH=0.15,Leverage=-0.1),200000);lv=mean(log(v));",
    );
    near(&e, "lv", &[-0.5], 0.02);
}
#[test]
fn estimate_seeded_fixtures_optimum_opg_covariance_and_summary() {
    let mut e = Engine::default();
    // Matches documented sample-moment presamples. arch default estimates use a
    // different backcast; strict optimum comparisons are against our SciPy script.
    for (constructor, data, want, ll, cov) in [
        (
            "garch(1,1)",
            include_str!("fixtures/econ/garch.csv"),
            GARCH_EST,
            GARCH_EST_LOGL,
            GARCH_COV,
        ),
        (
            "egarch(1,1)",
            include_str!("fixtures/econ/egarch.csv"),
            EGARCH_EST,
            EGARCH_EST_LOGL,
            EGARCH_COV,
        ),
        (
            "gjr(1,1)",
            include_str!("fixtures/econ/gjr.csv"),
            GJR_EST,
            GJR_EST_LOGL,
            GJR_COV,
        ),
        (
            "garch(1,1,'Distribution','t')",
            include_str!("fixtures/econ/garch_t.csv"),
            GARCH_T_EST,
            GARCH_T_EST_LOGL,
            GARCH_T_COV,
        ),
    ] {
        sample(&mut e, data);
        run(
            &mut e,
            &format!(
                "[m,c,l,i]=estimate({constructor},y,'Display','off');s=summarize(m);x=i.X;d=diag(c);li=infer(m,y);[vi,lli]=infer(m,y);"
            ),
        );
        near(&e, "x", want, 1e-4);
        near(&e, "l", &[ll], 1e-6);
        near(&e, "lli", &[ll], 1e-6);
        let d = vals(&e, "d");
        for (got, want) in d.iter().zip(cov) {
            assert!(
                (got - want).abs() < 0.01 * want.abs(),
                "OPG {got} != {want}"
            );
        }
        check(
            &mut e,
            &format!(
                "s.SampleSize==2000 && s.NumEstimatedParameters=={} && s.LogLikelihood==l && abs(s.AIC-(-2*l+2*{}))<1e-10 && istable(s.Table) && isequal(s.Table.Properties.VariableNames,{{'Value' 'StandardError' 'TStatistic' 'PValue'}}) && i.exitflag>0",
                want.len(),
                want.len()
            ),
        );
        let output = e.execute("summarize(m)").unwrap().output.join("\n");
        assert!(output.contains("Effective Sample Size: 2000"), "{output}");
        let ll_val: f64 = output
            .lines()
            .find(|l| l.contains("LogLikelihood:"))
            .and_then(|l| l.split(':').nth(1))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!((ll_val - vals(&e, "l")[0]).abs() < 1.0, "{output}");
    }
}
#[test]
fn fixed_parameters_starts_options_and_estimation_errors() {
    let mut e = Engine::default();
    sample(&mut e, include_str!("fixtures/econ/garch.csv"));
    run(
        &mut e,
        "m=garch(Constant=NaN,GARCH=0.82,ARCH=NaN,Offset=NaN);[fit,c,l,i]=estimate(m,y,Constant0=0.07,GARCH0=0.2,ARCH0=0.1,Offset0=0,Display='off');s=summarize(fit);",
    );
    check(
        &mut e,
        "fit.GARCH{1}==0.82 && c(2,2)==0 && all(c(2,:)==0) && all(c(:,2)==0) && s.NumEstimatedParameters==3 && numel(i.X)==4 && numel(i.X0)==4 && abs(i.X0(1)-0.07)<1e-12",
    );
    run(
        &mut e,
        "[fit,c,l,i]=estimate(garch(Constant=2),y,Display='off');s=summarize(fit);",
    );
    check(
        &mut e,
        "fit.Constant==2 && c==0 && s.NumEstimatedParameters==0",
    );
    run(
        &mut e,
        "m=garch(1,1);o=optimoptions('fmincon','Algorithm','sqp','MaxIterations',1,'Display','off');[fit,c,l,i]=estimate(m,y,Options=o,Display='off');",
    );
    check(&mut e, "i.exitflag==0");
    fails(
        &mut e,
        "estimate(m,y,'GARCH0',[0.2 0.3],'Display','off')",
        "coefficient count",
    );
    fails(
        &mut e,
        "estimate(m,y,'Constant0',-1,'Display','off')",
        "positive scalar",
    );
    fails(&mut e, "estimate(m,y,'Display','bogus')", "Invalid Display");
    fails(&mut e, "estimate(m,[y y],'Display','off')", "column vector");
    fails(&mut e, "estimate(m,y,'E0',[],'Display','off')", "presample");
    run(&mut e, "m=garch(1,1);n=summarize(m);");
    check(&mut e, "isa(n,'garch') && isnan(n.Constant)");
}
#[test]
fn public_integration_helpers_and_cancellation() {
    use matrust::engine::econ_vol::{
        Kind, Model, conditional_variance, forecast_path, one_step_forecast,
    };
    // Obtain state through the public model interface without constructing private
    // estimation metadata or passing through the MATLAB text evaluator for kernels.
    let mut e = Engine::default();
    run(&mut e, "m=garch(Constant=0.1,GARCH=0.7,ARCH=0.2);");
    let v = matrust::engine::Value::from(e.array("m").unwrap());
    let m = matrust::engine::econ_vol::model_value(&e, &v).unwrap();
    let _: &Model = &m;
    assert_eq!(m.kind, Kind::Garch);
    assert!(matrust::engine::econ_vol::is_model(&e, &v));
    let c = AtomicBool::new(false);
    let variance = conditional_variance(&m, &[1., -1., 2.], Some(&[0.5]), Some(&[1.]), &c).unwrap();
    assert!((variance[0] - 0.85).abs() < 1e-14);
    let one = one_step_forecast(&m, &[0.5], &[1.]).unwrap();
    assert!((one - 0.85).abs() < 1e-14);
    assert_eq!(one, forecast_path(&m, 3, &[0.5], &[1.], &c).unwrap()[0]);
    let stopped = AtomicBool::new(true);
    assert!(
        conditional_variance(&m, &[1.], None, None, &stopped)
            .unwrap_err()
            .contains("cancel")
    );
    // Keep raising the flag until the call returns, so a start-of-call reset or
    // a slow timer cannot let the simulation finish uncancelled.
    let cancellation = e.cancellation();
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let handle = std::thread::spawn(move || {
        while !finished.load(std::sync::atomic::Ordering::Relaxed) {
            cancellation.store(true, std::sync::atomic::Ordering::Relaxed);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });
    let result = e.execute("simulate(m,5000000)");
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    handle.join().unwrap();
    assert!(result.unwrap_err().to_lowercase().contains("cancel"));
}
#[test]
fn speed_estimate_ten_thousand_points() {
    if let Ok(n) = std::env::var("ECON_VOL_BENCH_WORKERS") {
        matrust::initialize_worker_pool(n.parse().unwrap()).unwrap();
    }
    let mut e = Engine::default();
    run(
        &mut e,
        "rng(918);[v,y]=simulate(garch(Constant=0.08,GARCH=0.82,ARCH=0.12),10000);",
    );
    let begin = std::time::Instant::now();
    run(&mut e, "[m,c,l,i]=estimate(garch(1,1),y,Display='off');");
    let elapsed = begin.elapsed();
    eprintln!("estimate garch(1,1), n=10000, {:?}", elapsed);
    if !cfg!(debug_assertions) {
        assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
    }
}

#[test]
#[ignore = "release benchmark; run separately with ECON_VOL_BENCH_WORKERS=1 and N"]
fn release_benchmark_volatility_paths() {
    let workers: usize = std::env::var("ECON_VOL_BENCH_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    let mut e = Engine::default();
    run(
        &mut e,
        "rng(918);g=garch(Constant=0.08,GARCH=0.82,ARCH=0.12);eg=egarch(Constant=-0.05,GARCH=0.88,ARCH=0.18,Leverage=-0.14);j=gjr(Constant=0.04,GARCH=0.82,ARCH=0.06,Leverage=0.16);[v,y]=simulate(g,10000);[ve,ye]=simulate(eg,2000);[vj,yj]=simulate(j,2000);",
    );
    for (name, code) in [
        ("garch_infer_10000", "v=infer(g,y);"),
        ("garch_forecast_10000", "f=forecast(g,10000,y);"),
        ("garch_simulate_200000", "[vv,yy]=simulate(g,200000);"),
        (
            "garch_estimate_10000",
            "m=estimate(garch(1,1),y,Display='off');",
        ),
        (
            "egarch_estimate_2000",
            "m=estimate(egarch(1,1),ye,Display='off');",
        ),
        (
            "gjr_estimate_2000",
            "m=estimate(gjr(1,1),yj,Display='off');",
        ),
    ] {
        let mut times = Vec::new();
        for _ in 0..5 {
            let now = std::time::Instant::now();
            run(&mut e, code);
            times.push(now.elapsed().as_secs_f64());
        }
        times.sort_by(f64::total_cmp);
        eprintln!("workers={workers} {name} median_seconds={:.6}", times[2]);
    }
}
#[test]
fn tall_zero_column_data_and_presample_reject_fast() {
    let mut e = Engine::default();
    fails(
        &mut e,
        "infer(garch(Constant=1),zeros(50000000,0))",
        "nonempty",
    );
    fails(
        &mut e,
        "estimate(garch(1,1),zeros(50000000,0),Display='off')",
        "nonempty",
    );
    fails(
        &mut e,
        "infer(garch(Constant=0.1,GARCH=0.7,ARCH=0.2),[1;2],E0=zeros(50000000,0))",
        "presample",
    );
    fails(
        &mut e,
        "forecast(garch(Constant=0.1,GARCH=0.7,ARCH=0.2),1,[1;2],V0=zeros(50000000,0))",
        "presample",
    );
}
