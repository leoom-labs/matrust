//! Expected numerical values: tool/refs/econ/arima.py (NumPy/SciPy; never MATLAB).
//! Explicit presamples define a conditional likelihood. statsmodels AutoReg
//! agrees for AR fits; SARIMAX's stationary/diffuse initialization uses a
//! different likelihood, but its long-history fixed forecasts/MSE agree.
use matrust::{Array, Engine, Matrix};
use serde_json::Value as Json;
fn run(e: &mut Engine, s: &str) {
    e.execute(s).unwrap_or_else(|err| panic!("{s}: {err}"));
}
fn check(e: &mut Engine, s: &str) {
    run(e, &format!("assert({s});"));
}
fn fail(e: &mut Engine, s: &str, msg: &str) {
    let error = e.execute(s).unwrap_err();
    assert!(error.contains(msg), "{s}: {error}");
}
fn values(e: &Engine, n: &str) -> Vec<f64> {
    match e.array(n).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        v => panic!("{n}: {v:?}"),
    }
}
fn near(got: &[f64], want: &[f64], atol: f64, rtol: f64) {
    assert_eq!(got.len(), want.len());
    for (i, (&a, &b)) in got.iter().zip(want).enumerate() {
        assert!(
            (a - b).abs() <= atol + rtol * b.abs(),
            "[{i}] {a:0.16e} != {b:0.16e} (delta={:0.3e})",
            a - b
        );
    }
}
fn refs() -> Json {
    serde_json::from_str(include_str!("fixtures/econ/arima.json")).unwrap()
}
fn nums(v: &Json) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}
fn vector(v: &Json) -> String {
    format!(
        "[{}]",
        nums(v)
            .iter()
            .map(|x| format!("{x:0.17e}"))
            .collect::<Vec<_>>()
            .join(";")
    )
}

fn model(c: &Json, template: bool) -> String {
    let m = &c["model"];
    let number = |k: &str| {
        if template {
            "NaN".into()
        } else {
            format!("{}", m[k])
        }
    };
    let mut opts = vec![
        format!("'Constant',{}", number("c")),
        format!("'Variance',{}", number("v")),
        format!("'D',{}", m["d"]),
        format!("'Seasonality',{}", m["s"]),
    ];
    for k in ["ar", "sar", "ma", "sma"] {
        let v = nums(&m[k]);
        let active: Vec<_> = v.iter().enumerate().filter(|(_, x)| **x != 0.).collect();
        if !active.is_empty() {
            let name = k.to_ascii_uppercase();
            let l = active
                .iter()
                .map(|(i, _)| (i + 1).to_string())
                .collect::<Vec<_>>()
                .join(" ");
            let v = active
                .iter()
                .map(|(_, x)| {
                    if template {
                        "NaN".into()
                    } else {
                        format!("{x:0.17e}")
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            opts.push(format!("'{name}Lags',[{l}],'{name}',[{v}]"));
        }
    }
    if !m["beta"].as_array().unwrap().is_empty() {
        opts.push(format!(
            "'Beta',{}",
            if template {
                "NaN".into()
            } else {
                vector(&m["beta"])
            }
        ));
    }
    if m["t"] == true {
        opts.push(format!(
            "'Distribution',struct('Name','t','DoF',{})",
            if template {
                "NaN".into()
            } else {
                format!("{}", m["nu"])
            }
        ));
    }
    format!("arima({})", opts.join(","))
}
fn sample(e: &mut Engine, c: &Json) {
    // Host insertion respects the interpreter's bounded expression syntax;
    // large numerical fixtures are data, not 500-node source expressions.
    for (name, key) in [
        ("y", "y"),
        ("y0", "y0"),
        ("e0", "e0"),
        ("innov", "residual"),
    ] {
        let data = nums(&c[key]);
        e.insert_array(
            name,
            Array::Numeric(Matrix::from_column_vec(data.len(), 1, data).unwrap()),
        )
        .unwrap();
    }
    if let Some(rows) = c["x"].as_array() {
        let nr = rows.len();
        let nc = rows[0].as_array().unwrap().len();
        let data = (0..nc)
            .flat_map(|j| rows.iter().map(move |r| r[j].as_f64().unwrap()))
            .collect();
        e.insert_array(
            "x",
            Array::Numeric(Matrix::from_column_vec(nr, nc, data).unwrap()),
        )
        .unwrap();
    }
}
fn xoption(c: &Json) -> &'static str {
    if c["x"].is_null() { "" } else { ",'X',x" }
}
#[test]
fn constructor_properties_seasonal_degrees_lags_and_value_semantics() {
    let mut e = Engine::default();
    run(&mut e, "m=arima(1,1,1); a=m; m.Constant=2; p=m.P; q=m.Q;");
    check(
        &mut e,
        "p==2 && q==1 && isnan(a.Constant) && m.Constant==2 && isa(m,'arima') && strcmp(class(m),'arima')",
    );
    run(
        &mut e,
        "m=arima(AR={0.4},MA={0.2},SAR={0.3},SMA={-0.4},SARLags=12,SMALags=12,D=1,Seasonality=12,Constant=0,Variance=1);",
    );
    check(
        &mut e,
        "m.P==26 && m.Q==13 && numel(m.SAR)==12 && m.SAR{12}==0.3 && m.SAR{1}==0",
    );
    run(
        &mut e,
        "m=arima('ARLags',[3 1],'AR',{0.1 0.2},'MA',0.3,'Variance',1,'Constant',0);m.AR{2}=0.1;b=m;m.AR{1}=0.25;",
    );
    check(
        &mut e,
        "m.P==3 && m.AR{3}==0.1 && b.AR{1}==0.2 && m.AR{1}==0.25",
    );
    run(
        &mut e,
        "m.Distribution='t';m.Distribution.DoF=7;m.Description='my model';m.SeriesName='returns';",
    );
    check(
        &mut e,
        "m.Distribution.DoF==7 && strcmp(m.Description,'my model') && strcmp(m.SeriesName,'returns')",
    );
    let out = e.execute("m=arima(1,1,1)").unwrap();
    assert!(format!("{out:?}").contains("ARIMA(1,1,1) Model (Gaussian Distribution)"));
    for (s, msg) in [
        ("arima(-1,0,0)", "InvalidOrder"),
        ("arima(1,0.5,0)", "InvalidOrder"),
        ("arima(257,0,0)", "InvalidOrder"),
        ("arima(ARLags=[1 1])", "InvalidLags"),
        ("arima(ARLags=[0 2])", "InvalidLags"),
        ("arima(ARLags=[1 3],AR=0.2)", "LagSizeMismatch"),
        ("arima(AR=1)", "stationary"),
        ("arima(MA=-1)", "invertible"),
        ("arima(Variance=0)", "positive"),
        ("arima(Distribution=struct('Name','t','DoF',2))", "exceed 2"),
        ("arima(uifigure=1)", "Unknown option"),
        ("m.P=1", "read-only"),
        ("m.AR={2}", "stationary"),
        ("m.Q=3", "read-only"),
        (
            "m.Variance=struct('Name','garch')",
            "Variance must be a positive scalar",
        ),
    ] {
        fail(&mut e, s, msg);
    }
}
#[test]
fn fixed_conditional_residuals_loglikelihood_and_variance_against_scipy() {
    let r = refs();
    for c in r["cases"].as_array().unwrap() {
        let mut e = Engine::default();
        sample(&mut e, c);
        run(
            &mut e,
            &format!(
                "m={};[z,v,ll]=infer(m,y,'Y0',y0,'E0',e0{});",
                model(c, false),
                xoption(c)
            ),
        );
        near(&values(&e, "z"), &nums(&c["residual"]), 1e-10, 0.);
        near(
            &values(&e, "ll"),
            &[c["fixed_logl"].as_f64().unwrap()],
            1e-10,
            0.,
        );
        near(&values(&e, "v"), &vec![0.7; 500], 0., 0.);
    }
}
#[test]
fn airline_log_series_seasonal_fixed_reference() {
    let mut e = Engine::default();
    let vals: Vec<_> = include_str!("fixtures/econ/airline.csv").lines().collect();
    assert_eq!(vals.len(), 144);
    run(
        &mut e,
        &format!(
            "y=log([{}]);m=arima('Constant',0,'Variance',0.002,'D',1,'Seasonality',12,'MA',-0.3,'SMALags',12,'SMA',-0.4);[z,v,ll]=infer(m,y(14:end),'Y0',y(1:13),'E0',zeros(13,1));",
            vals.join(";")
        ),
    );
    let r = refs();
    near(
        &values(&e, "z"),
        &nums(&r["airline"]["residual"]),
        1e-10,
        0.,
    );
    near(
        &values(&e, "ll"),
        &[r["airline"]["logl"].as_f64().unwrap()],
        1e-10,
        0.,
    );
}
#[test]
fn conditional_estimates_and_opg_covariance_seven_families() {
    let r = refs();
    for c in r["cases"].as_array().unwrap() {
        let mut e = Engine::default();
        sample(&mut e, c);
        let mut starts = vec![
            format!("'Constant0',{}", c["model"]["c"]),
            format!("'Variance0',{}", c["model"]["v"]),
        ];
        for k in ["ar", "sar", "ma", "sma", "beta"] {
            let active: Vec<_> = nums(&c["model"][k])
                .into_iter()
                .filter(|v| *v != 0.)
                .collect();
            if !active.is_empty() {
                starts.push(format!(
                    "'{}0',[{}]",
                    if k == "beta" {
                        "Beta".into()
                    } else {
                        k.to_ascii_uppercase()
                    },
                    active
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
        }
        if c["model"]["t"] == true {
            starts.push("'DoF0',7.5".into());
        }
        run(
            &mut e,
            &format!(
                "m={};[f,cov,ll,info]=estimate(m,y,'Y0',y0,'E0',e0,'Display','off',{}{});params=info.X;dg=diag(cov);s=summarize(f);",
                model(c, true),
                starts.join(","),
                xoption(c)
            ),
        );
        eprintln!(
            "{} params {:?}, logL {:?}",
            c["model"]["name"],
            values(&e, "params"),
            values(&e, "ll")
        );
        near(&values(&e, "params"), &nums(&c["opt"]), 1e-4, 1e-4);
        near(&values(&e, "ll"), &[c["logl"].as_f64().unwrap()], 1e-6, 0.);
        near(&values(&e, "dg"), &nums(&c["covdiag"]), 1e-8, 0.01);
        check(
            &mut e,
            "s.SampleSize==500 && s.NumEstimatedParameters==numel(params) && istable(s.Table) && istable(s.VarianceTable)",
        );
        run(
            &mut e,
            "[ai,bi]=aicbic(ll,s.NumEstimatedParameters,s.SampleSize);",
        );
        check(
            &mut e,
            "abs(s.AIC-ai)<1e-12 && abs(s.BIC-bi)<1e-12 && info.exitflag>0",
        );
    }
}
#[test]
fn forecasts_and_integrated_psi_mse_against_scipy_and_sarimax() {
    let r = refs();
    for c in r["cases"].as_array().unwrap() {
        let mut e = Engine::default();
        sample(&mut e, c);
        run(
            &mut e,
            &format!(
                "m={};[yf,mse,v]=forecast(m,8,y,'E0',{}{});[yf2,mse2]=forecast(m,8,'Y0',y,'E0',{}{});",
                model(c, false),
                "innov",
                if c["x"].is_null() {
                    ""
                } else {
                    ",'XF',ones(8,1)"
                },
                "innov",
                if c["x"].is_null() {
                    ""
                } else {
                    ",'XF',ones(8,1)"
                }
            ),
        );
        near(&values(&e, "yf"), &nums(&c["forecast"]), 1e-10, 0.);
        near(&values(&e, "mse"), &nums(&c["mse"]), 1e-10, 0.);
        check(&mut e, "isequal(yf,yf2) && isequal(mse,mse2)");
    }
    let mut e = Engine::default();
    run(
        &mut e,
        "m=arima(Constant=0,AR=0.5,Variance=2);[y,s]=forecast(m,3,[2 4]);",
    );
    near(&values(&e, "y"), &[1., 0.5, 0.25, 2., 1., 0.5], 1e-14, 0.);
    near(
        &values(&e, "s"),
        &[2., 2.5, 2.625, 2., 2.5, 2.625],
        1e-14,
        0.,
    );
    run(
        &mut e,
        "m=arima(Constant=1,D=1,Variance=2);[y,s]=forecast(m,3,10);",
    );
    near(&values(&e, "y"), &[11., 12., 13.], 0., 0.);
    near(&values(&e, "s"), &[2., 4., 6.], 0., 0.);
}
#[test]
fn simulation_session_stream_reproducibility_and_theoretical_moments() {
    let mut e = Engine::with_limits(matrust::engine::Limits::local());
    run(
        &mut e,
        "m=arima(Constant=1,AR=0.6,Variance=2);rng(19);[a,z,v]=simulate(m,200000);rng(19);b=simulate(m,200000);",
    );
    check(&mut e, "isequal(a,b) && all(v==2)");
    let y = values(&e, "a");
    let mean = y.iter().sum::<f64>() / y.len() as f64;
    let variance = y.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / y.len() as f64;
    let acf = y
        .windows(2)
        .map(|v| (v[0] - mean) * (v[1] - mean))
        .sum::<f64>()
        / ((y.len() - 1) as f64 * variance);
    assert!((mean - 2.5).abs() < 0.04, "mean {mean}");
    assert!((variance - 3.125).abs() < 0.08, "var {variance}");
    assert!((acf - 0.6).abs() < 0.015, "ACF {acf}");
    run(
        &mut e,
        "m=arima(Constant=0,Variance=1,Distribution=struct('Name','t','DoF',8));rng(5);a=simulate(m,200000);rng(5);b=simulate(m,200000);",
    );
    check(&mut e, "isequal(a,b)");
    let y = values(&e, "a");
    let var = y.iter().map(|v| v * v).sum::<f64>() / y.len() as f64;
    assert!((var - 1.).abs() < 0.04, "t variance {var}");
    run(
        &mut e,
        "m=arima(Constant=0,Variance=1);rng(9);a=simulate(m,5,NumPaths=3);rng(9);b=randn(5,3);",
    );
    check(&mut e, "isequal(a,b)");
}
#[test]
fn fixed_parameters_missing_values_presample_paths_options_and_summaries() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=arima(AR=0.5,Constant=0);[f,c,ll,info]=estimate(m,[1;2;NaN;3;2;1],Y0=0,Display='off');s=summarize(f);z=s.Table.StandardError;",
    );
    check(
        &mut e,
        "s.SampleSize==5 && s.NumEstimatedParameters==1 && all(all(c(1:2,:)==0)) && all(all(c(:,1:2)==0)) && all(z==0)",
    );
    run(
        &mut e,
        "m=arima(Constant=0,AR=0.5,Variance=1);[z,v,ll]=infer(m,[1 2;3 4],Y0=[NaN;2;4]);",
    );
    near(&values(&e, "z"), &[-1., 2.5, 0., 3.], 0., 0.);
    run(
        &mut e,
        "m=arima(Constant=0,MA=0.5,Variance=1);z=infer(m,[1;2;3],E0=[999;NaN;0.2]);",
    );
    near(&values(&e, "z"), &[0.9, 1.55, 2.225], 1e-14, 0.);
    run(&mut e, "m=arima(1,0,0);r=summarize(m);");
    check(&mut e, "isa(r,'arima') && isnan(r.AR{1})");
    let out = e.execute("summarize(m);").unwrap();
    let text = format!("{out:?}");
    assert!(text.contains("ARIMA(1,0,0)"));
    assert!(
        text.contains("P: 1") && text.contains("Variance: NaN"),
        "{text}"
    );
    for (s, msg) in [
        ("infer(arima(1,0,0),[1;2])", "fully specified"),
        ("estimate(arima(2,0,1),1,'Display','off')", "Too few"),
        ("infer(arima(AR=0.2,Constant=0,Variance=1),[1;2],Y0=[])", ""),
        ("forecast(arima(AR=0.2,Constant=0,Variance=1),2)", "Y0"),
        (
            "simulate(arima(Constant=0,Variance=1),2,NumPaths=0)",
            "integer",
        ),
        (
            "estimate(m,[1;2;3],Display='off',uicontrol=1)",
            "Unknown option",
        ),
        (
            "infer(arima(AR=0.2,Constant=0,Variance=1),[1;2],Y0=zeros(0,2))",
            "",
        ),
        (
            "forecast(arima(AR={0.2 0.1},Constant=0,Variance=1),2,1)",
            "Y0",
        ),
    ] {
        if !msg.is_empty() {
            fail(&mut e, s, msg);
        }
    }
    run(
        &mut e,
        "m=arima(0,0,0);[f,c,ll,info]=estimate(m,[1;2;1;2;3;4],Display='off',Options=optimset('MaxIter',100,'TolFun',1e-8));",
    );
    check(&mut e, "isstruct(info.options) && numel(info.X)==2");
    fail(
        &mut e,
        "estimate(m,[1;2;3],Options=struct('uifigure',1))",
        "not supported",
    );
    for name in ["estimate", "forecast", "infer", "simulate", "summarize"] {
        let err = e.execute(&format!("{name}(42,2);")).unwrap_err();
        // Shared econometric method names report an undefined method for non-model input.
        assert!(err.contains("Undefined function"), "{name}: {err}");
    }
}
#[test]
fn aicbic_vectors_normalization_and_extra_criteria() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[a,b,ic]=aicbic([-10 -12],[2 3],100);[an,bn,icn]=aicbic([-10 -12],[2 3],100,Normalize=true);d=ic.aicc;",
    );
    near(&values(&e, "a"), &[24., 30.], 0., 0.);
    near(
        &values(&e, "b"),
        &[20. + 2. * 100_f64.ln(), 24. + 3. * 100_f64.ln()],
        1e-13,
        0.,
    );
    near(
        &values(&e, "d"),
        &[24. + 12. / 97., 30. + 24. / 96.],
        1e-13,
        0.,
    );
    check(
        &mut e,
        "isequal(an,a/100) && isequal(bn,b/100) && isequal(icn.caic,ic.caic/100) && isfield(ic,'hqc')",
    );
    for (s, msg) in [
        ("aicbic(-1,-1,10)", "nonnegative"),
        ("aicbic(-1,1,0)", "positive"),
        ("[a,b]=aicbic(-1,1)", "numObs"),
        ("aicbic([-1 -2],[1 2 3],10)", "compatible"),
    ] {
        fail(&mut e, s, msg);
    }
}
#[test]
fn estimate_arma21_ten_thousand_under_two_seconds() {
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let mut e = Engine::with_limits(matrust::engine::Limits::local());
            run(&mut e, "rng(99);truth=arima(Constant=0.1,AR={0.5 -0.2},MA=0.3,Variance=0.7);y=simulate(truth,10000);m=arima(2,0,1);");
            let start = std::time::Instant::now();
            run(&mut e, "[f,c,ll,info]=estimate(m,y,Y0=[0;0],Display='off',AR0=[0.5 -0.2],MA0=0.3);");
            let elapsed = start.elapsed();
            eprintln!("ARMA(2,1), n=10000, workers={workers}, elapsed={elapsed:?}");
            assert!(elapsed.as_secs_f64() < 2., "{elapsed:?}");
            check(&mut e, "info.exitflag>0 && isfinite(ll)");
        });
    }
}

#[test]
fn automatic_ar_backcast_and_forecast_innovation_defaults() {
    let mut e = Engine::default();
    // Gaussian AR(1) backward prediction: mu + phi*(y1-mu) = 3.
    // The independent conditional recursion is also in tool/refs/econ/arima.py.
    run(
        &mut e,
        "m=arima(Constant=1,AR=0.5,Variance=1);[z,v,ll]=infer(m,[4;1;3]);",
    );
    near(&values(&e, "z"), &[1.5, -2., 1.5], 1e-14, 0.);
    run(
        &mut e,
        "m=arima(Constant=0,AR=0.5,MA=0.2,Variance=1);y=[1;2;3;4];z=infer(m,y(2:end),Y0=y(1));a=forecast(m,3,y);b=forecast(m,3,y,E0=z);c=forecast(m,3,y(4));d=forecast(m,3,y(4),E0=0);",
    );
    check(&mut e, "isequal(a,b) && isequal(c,d)");
    fail(
        &mut e,
        "infer(arima(Constant=0,AR={0.3 0.1},Variance=1),[1;2],Y0=1)",
        "Y0",
    );
    fail(
        &mut e,
        "infer(arima(Constant=0,MA={0.3 0.1},Variance=1),[1;2],E0=1)",
        "E0",
    );
    run(
        &mut e,
        "try; infer(arima(Constant=0,Variance=1),[1;2],uifigure=1); catch exception; identifier=exception.identifier; end;",
    );
    check(&mut e, "strcmp(identifier,'econ:infer:UnknownOption')");
}
#[test]
fn forecast_x0_nan_synchronizes_y0_and_user_shadowed_constructor_runs_once() {
    let mut e = Engine::default();
    // Hand-derived reference (see arima.forecast Algorithms):
    // Y0=[1;2;3;4], X0=[0;0;NaN] aligned at the last row drops row 4 from both:
    // Y0 -> [1;2;3], X0 -> [0;0].
    // With Constant=0, AR=0.5, MA=0.2, Beta=1:
    //   e(1) = 2 - (0.5*1 + 0.2*0 + 1*0) = 1.5
    //   e(2) = 3 - (0.5*2 + 0.2*1.5 + 1*0) = 1.7
    // 1-step forecast with XF=0: 0.5*3 + 0.2*1.7 + 1*0 = 1.84.
    run(
        &mut e,
        "m=arima(Constant=0,AR=0.5,MA=0.2,Beta=1,Variance=1);f=forecast(m,1,[1;2;3;4],X0=[0;0;NaN],XF=0);",
    );
    near(&values(&e, "f"), &[1.84], 1e-14, 0.);

    // Local function garch() shadowing the volatility constructor must be called once
    // in summarize(garch()), [a,b]=infer(garch(),y), and nested summarize(estimate(garch(),y)).
    run(
        &mut e,
        "global calls
        calls = 0;
        y = [1; 2; 3; 4; 5];
        summarize(garch());
        c1 = calls;
        [a, b, ll] = infer(garch(), y);
        c2 = calls;
        summarize(estimate(garch(), y, Display='off'));
        c3 = calls;
        assert(numel(a) == 5 && numel(b) == 5 && isscalar(ll));
        function m = garch()
            global calls
            calls = calls + 1;
            m = arima(Constant=0, AR=0.5, Variance=1);
        end",
    );
    assert_eq!(values(&e, "c1"), vec![1.]);
    assert_eq!(values(&e, "c2"), vec![2.]);
    assert_eq!(values(&e, "c3"), vec![3.]);
}
