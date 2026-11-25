//! Independent statsmodels OLS/GLM and analytic model-object coverage.
use matrust::{Array, Engine};
use serde::Deserialize;

#[derive(Deserialize)]
struct Reference {
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    code: String,
    rows: usize,
    cols: usize,
    data: Vec<f64>,
    atol: f64,
    rtol: f64,
}
fn numbers(e: &Engine, name: &str) -> Vec<f64> {
    match e.workspace().get(name).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        other => panic!("{name}: unexpected {}", other.class()),
    }
}
fn run(e: &mut Engine, code: &str) {
    e.execute(code)
        .unwrap_or_else(|err| panic!("{code}: {err}"));
}
fn reference(names: &[&str]) {
    let fixture: Reference =
        serde_json::from_str(include_str!("fixtures/linear_models/reference.json")).unwrap();
    let mut checked = 0;
    for c in fixture
        .cases
        .into_iter()
        .filter(|c| names.contains(&c.name.as_str()))
    {
        checked += 1;
        let mut e = Engine::default();
        run(&mut e, &c.code);
        let ws = e.workspace();
        let actual = ws.get("zz").unwrap();
        assert_eq!(
            (actual.rows(), actual.cols()),
            (c.rows, c.cols),
            "{} shape",
            c.name
        );
        for (i, (a, b)) in numbers(&e, "zz").iter().zip(c.data).enumerate() {
            assert!(
                (a - b).abs() <= c.atol + c.rtol * b.abs(),
                "{}[{i}]: got {a:.17e}, expected {b:.17e}",
                c.name
            );
        }
    }
    assert_eq!(checked, names.len());
}
#[test]
fn ols_coefficients_and_inference() {
    reference(&["ols", "ols_statistics", "fitted_residual"]);
}
#[test]
fn no_intercept_reference() {
    reference(&["no_intercept"]);
}
#[test]
fn formula_interaction_and_square_reference() {
    reference(&["interaction", "colon", "squared"]);
}
#[test]
fn named_model_specifications() {
    reference(&[
        "spec_linear",
        "spec_interactions",
        "spec_quadratic",
        "spec_purequadratic",
    ]);
}
#[test]
fn table_response_and_predictor_names() {
    reference(&["table"]);
    let mut e = Engine::default();
    run(
        &mut e,
        "t=table([1;2;3;4],[3;5;6;9],[2;0;1;2],'VariableNames',{'weight','response','age'});m=fitlm(t,'response ~ weight + age');c=m.Coefficients;names=c.Properties.RowNames;cn=m.CoefficientNames;p=predict(m,table([2;3],[1;0],'VariableNames',{'age','weight'}));q=predict(m,[1 2;0 3]);cl=class(m);",
    );
    assert_eq!(numbers(&e, "p"), numbers(&e, "q"));
    let output = e.execute("names\ncn\ncl").unwrap().output.join("");
    assert!(
        output.contains("weight")
            && output.contains("age")
            && output.contains("(Intercept)")
            && output.contains("LinearModel"),
        "{output}"
    );
}
#[test]
fn mean_prediction_confidence_intervals() {
    reference(&["predict"]);
}
#[test]
fn binomial_irls_reference() {
    reference(&["glm_binomial", "glm_binomial_stats", "glm_binomial_predict"]);
}
#[test]
fn poisson_irls_reference() {
    reference(&["glm_poisson", "glm_poisson_stats", "glm_poisson_predict"]);
}
#[test]
fn normal_and_optional_probit_link() {
    reference(&[
        "glm_normal",
        "glm_normal_stats",
        "glm_normal_predict",
        "glm_probit",
        "glm_probit_stats",
        "glm_probit_predict",
    ]);
}
#[test]
fn stepwise_selects_main_effects_and_interaction() {
    let mut rows = Vec::new();
    let mut response = Vec::new();
    for repetition in 0..3 {
        for x1 in [-1., 1.] {
            for x2 in [-1., 1.] {
                for x3 in [-1., 1.] {
                    rows.push(format!("{x1} {x2} {x3}"));
                    response.push(
                        (2. + 3. * x1 + 1.5 * x2 + 0.8 * x1 * x2 + 0.1 * (repetition as f64 - 1.))
                            .to_string(),
                    );
                }
            }
        }
    }
    let setup = format!("X=[{}];y=[{}];", rows.join(";"), response.join(";"));
    let mut e = Engine::default();
    run(&mut e, &setup);
    let out = e
        .execute("m=stepwiselm(X,y);c=m.Coefficients;b=c.Estimate;f=m.Formula;")
        .unwrap()
        .output
        .join("");
    assert!(
        out.contains("1. Adding x1") && out.contains("Adding x2") && out.contains("Adding x1:x2"),
        "{out}"
    );
    run(
        &mut e,
        "r=fitlm(X,y,'y ~ x1*x2');rc=r.Coefficients;expected=rc.Estimate;",
    );
    let actual = numbers(&e, "b");
    assert_eq!(actual.len(), 4);
    for (a, b) in actual.iter().zip(numbers(&e, "expected")) {
        assert!((a - b).abs() < 1e-12);
    }
    for (a, b) in actual.iter().zip([2., 3., 1.5, 0.8]) {
        assert!((a - b).abs() < 1e-12);
    }
    let out = e
        .execute(
            "back=stepwiselm(X,y,'linear','Upper','linear');bc=back.Coefficients;bb=bc.Estimate;",
        )
        .unwrap()
        .output
        .join("");
    assert!(out.contains("Removing x3"), "{out}");
    assert_eq!(numbers(&e, "bb").len(), 3);
}
#[test]
fn model_display_and_value_semantics() {
    let mut e = Engine::default();
    let out = e
        .execute("m=fitlm([0;1;2;3;4],[1;3;4;8;9]);a=m;m")
        .unwrap()
        .output
        .join("");
    for text in [
        "Linear regression model:",
        "y ~ 1 + x1",
        "Estimated Coefficients:",
        "Estimate",
        "SE",
        "tStat",
        "pValue",
        "Number of observations: 5, Error degrees of freedom: 3",
        "Root Mean Squared Error:",
        "R-squared:",
        "Adjusted R-Squared:",
        "F-statistic vs. constant model:",
    ] {
        assert!(out.contains(text), "missing {text}: {out}");
    }
    assert!(e.execute("m.RMSE=2;").unwrap_err().contains("read-only"));
    assert!(e.execute("delete(m)").unwrap_err().contains("unsupported"));
    run(
        &mut e,
        "clear m;p=predict(a,[1;2]);q=a.predict([1;2]);is=isa(a,'LinearModel');",
    );
    assert_eq!(numbers(&e, "p"), numbers(&e, "q"));
    let nested = e
        .execute("saved={a};clear a;saved")
        .unwrap()
        .output
        .join("");
    assert!(nested.contains("[1x1 LinearModel]"), "{nested}");
    run(&mut e, "p=predict(saved{1},[1;2]);");
    assert_eq!(numbers(&e, "p"), numbers(&e, "q"));
    let out = e
        .execute("g=fitglm([-2;-1;0;1;2;3],[0;1;0;1;0;1],'Distribution','binomial');g")
        .unwrap()
        .output
        .join("");
    assert!(out.contains("Generalized linear regression model:") && out.contains("Dispersion:"));
}
#[test]
fn clear_errors_for_unsupported_syntax_and_distribution() {
    let mut e = Engine::default();
    for formula in [
        "y ~ sin(x1)",
        "y ~ x1/x2",
        "y ~ x1^3",
        "y ~ x1 +",
        "y ~ x1 - x2",
        "z ~ x1",
        "y ~ x1 ** x2",
        "y ~ x 1",
    ] {
        let err = e
            .execute(&format!("fitlm([1 2;2 1;3 4;4 2],[1;3;2;5],'{formula}')"))
            .unwrap_err();
        assert!(err.contains("formula"), "{formula}: {err}");
    }
    assert!(
        e.execute("fitglm([1;2;3],[1;2;3],'Distribution','gamma')")
            .unwrap_err()
            .contains("unsupported distribution")
    );
    assert!(
        e.execute("fitglm([1;2;3],[1;2;3],'Distribution','poisson','Link','logit')")
            .unwrap_err()
            .contains("combination")
    );
    let err = e.execute("predict(makedist('Normal'),1)").unwrap_err();
    assert!(
        err.contains("predict: unsupported model type prob.NormalDistribution"),
        "{err}"
    );
    let err = e.execute("[p,ci]=predict(1,1)").unwrap_err();
    assert!(err.contains("unsupported model type double"), "{err}");
}
#[test]
fn missing_rows_rank_deficiency_and_prediction_shape() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=fitlm([0;1;NaN;3;4],[1;3;5;8;9]);n=m.NumObservations;f=m.Fitted;r=m.Residuals;raw=r.Raw;[p,ci]=predict(m,[NaN;2]);",
    );
    assert_eq!(numbers(&e, "n"), [4.]);
    assert!(numbers(&e, "f")[2].is_nan());
    assert!(numbers(&e, "raw")[2].is_nan());
    assert!(numbers(&e, "p")[0].is_nan());
    assert!(numbers(&e, "ci")[0].is_nan());
    let out = e
        .execute("d=fitlm([0 0;1 2;2 4;3 6],[1;3;4;8]);[p,ci]=predict(d,[2 4]);")
        .unwrap()
        .output
        .join("");
    assert!(out.contains("rank deficient"));
    assert!(numbers(&e, "ci").iter().all(|v| v.is_nan()));
    assert!(
        e.execute("predict(m,[1 2])")
            .unwrap_err()
            .contains("columns")
    );
    assert!(e.execute("fitlm([1;Inf;3],[1;2;3])").is_err());
    assert!(
        e.execute("fitlm([1;2],[1;2])")
            .unwrap_err()
            .contains("degrees of freedom")
    );
    assert!(
        e.execute("fitlm(zeros(100,257),zeros(100,1))")
            .unwrap_err()
            .contains("too large")
    );
}
#[test]
fn help_pages_and_examples_are_registered() {
    for name in ["fitlm", "fitglm", "stepwiselm", "predict"] {
        assert!(matrust::session::FUNCTIONS.contains(&name));
        assert!(matrust::help::summary(name).is_some());
        let out = Engine::default()
            .execute(&format!("help {name}"))
            .unwrap()
            .output
            .join("");
        assert!(
            out.contains("Syntax:")
                && out.contains("Example:")
                && out.contains("Differences from MATLAB:")
        );
    }
}

#[test]
fn typed_inputs_string_options_and_unused_missing_predictors() {
    let mut e = Engine::default();
    run(
        &mut e,
        "spec=\"y ~ -1 + x1\";m=fitlm(single([1;2;3;4]),int32([2;4;5;9]),spec);c=m.Coefficients;b=c.Estimate;[p,ci]=predict(m,single([2;3]));r=fitlm([1;2;3;4],[2;4;5;9],'Intercept',false);rc=r.Coefficients;rb=rc.Estimate;",
    );
    assert_eq!(numbers(&e, "b"), numbers(&e, "rb"));
    run(
        &mut e,
        "d=\"poisson\";g=fitglm(single([-1;0;1;2;3]),int32([0;1;2;2;5]),'Distribution',d);cl=class(g);",
    );
    run(
        &mut e,
        "X=[1 NaN;2 NaN;3 NaN;4 NaN];m=fitlm(X,[2;4;5;9],'y ~ x1');n=m.NumObservations;p=predict(m,table([2;3],'VariableNames',{'x1'}));q=predict(m,[2 NaN;3 NaN]);",
    );
    assert_eq!(numbers(&e, "n"), [4.]);
    assert_eq!(numbers(&e, "p"), numbers(&e, "q"));
}

#[test]
fn model_bounds_output_budget_and_invalid_responses() {
    let mut e = Engine::default();
    for code in [
        "fitglm([1;2;3;4],[0;2;0;1],'Distribution','binomial')",
        "fitglm([1;2;3;4],[0;1.5;0;1],'Distribution','poisson')",
        "fitlm(ones(2,2,2),[1;2])",
        "fitlm([1;2;3],ones(2,2))",
        "stepwiselm([1;2;3;4],[1;2;3;4],'PEnter',.2,'PRemove',.1)",
        "[a,b,c]=predict(fitlm([1;2;3;4],[1;2;4;5]),[1;2])",
    ] {
        assert!(e.execute(code).is_err(), "{code}");
    }
    let mut limits = matrust::engine::Limits::browser();
    limits.output_entries = Some(1);
    let mut e = Engine::with_limits(limits);
    let err = e
        .execute(
            "X=[-1 -1;-1 1;1 -1;1 1;-1 -1;-1 1;1 -1;1 1];y=[-4;-1;2;7;-3;0;3;6];m=stepwiselm(X,y);",
        )
        .unwrap_err();
    assert!(err.contains("output exceeds"), "{err}");
}

#[test]
fn noncanonical_links_and_response_scale_intervals() {
    reference(&[
        "glm_normal_log",
        "glm_normal_log_stats",
        "glm_normal_log_predict",
        "glm_poisson_identity",
        "glm_poisson_identity_stats",
        "glm_poisson_identity_predict",
        "glm_binomial_ci",
        "glm_poisson_ci",
        "glm_probit_ci",
    ]);
}

#[test]
fn default_table_response_and_constant_model() {
    reference(&["table_default", "constant", "constant_table"]);
}

#[test]
fn stepwise_skips_saturated_candidates() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=stepwiselm([0 1;1 -1;2 0],[1;3;5]);c=m.Coefficients;b=c.Estimate;",
    );
    let b = numbers(&e, "b");
    assert_eq!(b.len(), 2);
    assert!((b[0] - 1.).abs() < 1e-12 && (b[1] - 2.).abs() < 1e-12);
}

#[test]
fn review_glm_rsquared_has_all_documented_fields() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=fitglm(zeros(4,1),[0;1;0;1],'constant','Distribution','binomial'); r=m.Rsquared; assert(abs(r.LLR)<1e-12); assert(abs(r.Deviance)<1e-12); assert(abs(r.AdjGeneralized)<1e-12); assert(abs(r.Ordinary)<1e-12); assert(abs(r.Adjusted)<1e-12); assert(numel(fieldnames(r))==5);",
    );
    reference(&[
        "glm_binomial_r2",
        "glm_probit_r2",
        "glm_poisson_r2",
        "glm_normal_r2",
        "glm_normal_log_r2",
        "glm_poisson_identity_r2",
        "glm_binomial_no_intercept_r2",
    ]);
}

#[test]
#[ignore = "optimized release timing probe; run with MATRUST_BENCH_WORKERS=1 and 4"]
fn benchmark_review_numerical_paths() {
    let workers = std::env::var("MATRUST_BENCH_WORKERS")
        .unwrap_or("1".into())
        .parse()
        .unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    let mut e = Engine::default();
    run(
        &mut e,
        "sx=[-32:-1 1:32]'; sy=[ones(32,1);2*ones(32,1)]; gx=linspace(-2,2,256)'; gy=double(mod((1:256)',3)==0); cx=(1:512)'; cy=[ones(256,1);2*ones(256,1)]; cm=fitcknn(cx,cy); bx=(1:200000)'; bg=mod(bx,8); TT=timetable(seconds([0:65535]'),sin([0:65535]')); tq=seconds([.5:2:65534.5]');",
    );
    for (name, source) in [
        (
            "linear_svm_64x1_scale10",
            "m=fitcsvm(sx,sy,'KernelScale',10); [p,s]=predict(m,sx);",
        ),
        (
            "stratified_kfold_512x1",
            "rng(1); cv=crossval(cm,'KFold',3); [p,s]=kfoldPredict(cv);",
        ),
        (
            "glm_binomial_256x1_rsquared",
            "m=fitglm(gx,gy,'Distribution','binomial'); r=m.Rsquared;",
        ),
        ("boxplot_200000_rows_8_groups", "h=boxplot(bx,bg);"),
        ("retime_65536_rows_32768_bins", "R=retime(TT,tq,'sum');"),
    ] {
        run(&mut e, source);
        let start = std::time::Instant::now();
        for _ in 0..10 {
            run(&mut e, source);
        }
        eprintln!(
            "{name}: workers={workers}, iterations=10, elapsed_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.
        );
    }
}
