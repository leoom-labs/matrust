//! Boundary regressions against statsmodels 0.15.0 / arch 9.0.0.
//! Regenerate with bash tool/ref-python.sh tool/refs/econ/boundary.py.
//! SARIMAX uses a known initial state with covariance R Q R', no likelihood
//! burn-in: CONDITIONAL likelihood with Y0=E0=0, matching estimate, rather than
//! the default exact stationary likelihood. arch uses backcast=mean(y^2).
use matrust::{Array, Engine, Matrix};
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/econ/boundary.json")).unwrap()
}
fn run(e: &mut Engine, source: &str) {
    e.execute(source)
        .unwrap_or_else(|err| panic!("{source}: {err}"));
}
fn values(e: &Engine, name: &str) -> Vec<f64> {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        a => panic!("{name}: {a:?}"),
    }
}
fn data(e: &mut Engine, c: &Value) {
    let y: Vec<_> = c["y"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    e.insert_array(
        "y",
        Array::Numeric(Matrix::from_column_vec(y.len(), 1, y).unwrap()),
    )
    .unwrap();
}
fn check_case(index: usize) {
    let refs = fixture();
    let c = &refs["cases"][index];
    let mut e = Engine::default();
    data(&mut e, c);
    let (model, presample) = match c["kind"].as_str().unwrap() {
        "garch" => ("garch(1,1)", ""),
        "ar" => ("arima(Constant=0,AR=NaN)", ",Y0=0,E0=0"),
        "ma" => ("arima(Constant=0,MA=NaN)", ",Y0=0,E0=0"),
        _ => unreachable!(),
    };
    run(
        &mut e,
        &format!("[m,c,ll,info]=estimate({model},y,Display='off'{presample});p=info.X;"),
    );
    let ours = values(&e, "ll")[0];
    let reference = c["ll"].as_f64().unwrap();
    eprintln!(
        "{} seed={}: reference ll={reference:.12}, ours={ours:.12}",
        c["name"], c["seed"]
    );
    assert!(
        ours >= reference - 1e-4,
        "{}: {ours} < {reference}",
        c["name"]
    );
    if ours > reference + 1e-4 {
        eprintln!("Reference boundary/nonconvergence stop: {}", c["optimizer"]);
        assert!(
            !c["converged"].as_bool().unwrap() || c["boundary"].as_bool().unwrap(),
            "Unexpectedly beat a converged interior reference: {reference} vs {ours}"
        );
    }
    let p = values(&e, "p");
    if c["converged"].as_bool().unwrap() && !c["boundary"].as_bool().unwrap() {
        for (got, reference) in p.iter().zip(c["params"].as_array().unwrap()) {
            assert!(
                (got - reference.as_f64().unwrap()).abs() <= 1e-3,
                "{}: parameters {p:?}, reference {}",
                c["name"],
                c["params"]
            );
        }
    }
    if c["kind"] == "garch" {
        assert!(
            p[0] > 0. && p[1] >= 0. && p[2] >= 0. && p[1] + p[2] <= 1. - 1e-7 + 1e-10,
            "{p:?}"
        );
        run(&mut e, "[v,li]=infer(m,y);assert(all(v>0));");
        if index == 0 {
            assert!(p[2] < 1e-5, "ARCH should be on its lower bound: {p:?}");
        }
    } else {
        assert!(p[1].abs() <= 1. - 1e-7 + 1e-10 && p[2] > 0., "{p:?}");
        run(&mut e, "[z,v,li]=infer(m,y,Y0=0,E0=0);assert(all(v>0));");
    }
    assert!((values(&e, "li")[0] - ours).abs() < 1e-8);
}
#[test]
fn zero_arch_garch() {
    check_case(0);
}
#[test]
fn near_unit_root_ar() {
    check_case(1);
}
#[test]
fn overdifferenced_ma() {
    check_case(2);
}
// NumPy default_rng(0), n=300: main's BFGS returned flag 2, ARCH=2.77e-13,
// ll=-431.15918916977387, versus arch's boundary ll=-430.8618051952996.
#[test]
fn stalled_seed_0() {
    check_case(3);
}
// NumPy default_rng(3), n=300: main's BFGS returned flag 2, ARCH=3.09e-18,
// ll=-429.6630926097514, versus arch's boundary ll=-429.5442660419509.
#[test]
fn stalled_seed_3() {
    check_case(4);
}
// NumPy default_rng(6), n=300: main's BFGS returned flag 2, ARCH=6.49e-16,
// ll=-425.6858181487967, versus arch's boundary ll=-424.4955130771267.
#[test]
fn stalled_seed_6() {
    check_case(5);
}

#[test]
fn fmincon_and_struct_options_all_families() {
    let mut e = Engine::default();
    data(&mut e, &fixture()["cases"][0]);
    for model in [
        "arima(0,0,0)",
        "arima(Constant=0,AR=NaN,Variance=garch(1,1))",
        "garch(1,1)",
        "egarch(1,1)",
        "gjr(1,1)",
    ] {
        for algorithm in ["sqp", "interior-point", "active-set"] {
            for options in [
                format!(
                    "optimoptions(@fmincon,Algorithm='{algorithm}',ConstraintTolerance=1e-6,OptimalityTolerance=1e-7,StepTolerance=1e-9,MaxIterations=2,MaxFunctionEvaluations=200,Display='off')"
                ),
                format!(
                    "struct('Algorithm','{algorithm}','ConstraintTolerance',1e-6,'OptimalityTolerance',1e-7,'StepTolerance',1e-9,'MaxIterations',2,'MaxFunctionEvaluations',200,'Display','off')"
                ),
            ] {
                run(
                    &mut e,
                    &format!(
                        "[m,c,l,i]=estimate({model},y,Options={options},Display='off');assert(isfinite(l));"
                    ),
                );
            }
        }
    }
    // The regARIMA path is an ARIMA error model with predictors; explicit Y0
    // leaves the separate X-without-Y0 presample rule outside this task.
    run(
        &mut e,
        "x=(1:numel(y))';[m,c,l,i]=estimate(arima(Beta=NaN,AR=NaN),y,X=x,Y0=0,Options=optimoptions(@fmincon,Algorithm='sqp'),Display='off');assert(isfinite(l));",
    );
    for model in ["arima(0,0,0)", "garch(1,1)"] {
        for opts in [
            "struct('Algorithm','uifigure')",
            "struct('ConstraintTolerance',0)",
            "struct('ConstraintTolerance',NaN)",
            "struct('Display','uifigure')",
            "struct('uicontrol',1)",
        ] {
            assert!(
                e.execute(&format!(
                    "estimate({model},y,Options={opts},Display='off');"
                ))
                .is_err(),
                "{model}, {opts}"
            );
        }
        run(
            &mut e,
            &format!(
                "[m,c,l,i]=estimate({model},y,Options=struct('MaxFunctionEvaluations',1),Display='off');assert(i.exitflag==0);assert(max(abs(i.X-i.X0))<1e-12);"
            ),
        );
    }
}

#[test]
fn constraint_tolerance_controls_active_polynomial_and_linear_boundaries() {
    let mut e = Engine::default();
    data(&mut e, &fixture()["cases"][2]);
    for tolerance in [1e-3, 1e-5] {
        run(
            &mut e,
            &format!(
                "[m,c,l,i]=estimate(arima(Constant=0,MA=NaN),y,E0=0,Options=struct('ConstraintTolerance',{tolerance}),Display='off');p=i.X;"
            ),
        );
        let p = values(&e, "p");
        assert!(
            p[1].abs() <= 1. - tolerance + 1e-10 && p[1] < -0.99,
            "{tolerance}: {p:?}"
        );
    }
    data(&mut e, &fixture()["cases"][3]);
    for tolerance in [1e-3, 1e-5] {
        run(
            &mut e,
            &format!(
                "[m,c,l,i]=estimate(garch(1,1),y,Options=struct('ConstraintTolerance',{tolerance}),Display='off');p=i.X;"
            ),
        );
        let p = values(&e, "p");
        assert!(
            p[1] + p[2] <= 1. - tolerance + 1e-10 && p[1] + p[2] > 0.99,
            "{tolerance}: {p:?}"
        );
    }
    // A fixed ARCH coefficient still contributes to both GJR linear rows;
    // the optimized leverage can be negative, but cannot cross -ARCH.
    run(
        &mut e,
        "[m,c,l,i]=estimate(gjr(Constant=NaN,GARCH=0.7,ARCH=0.1,Leverage=NaN),y,Display='off');p=i.X;assert(p(1)>0 && p(3)+p(4)>=-1e-10 && p(2)+p(3)+0.5*p(4)<=1-1e-7+1e-10);[v,li]=infer(m,y);assert(all(v>0));",
    );
    run(
        &mut e,
        "[m,c,l,i]=estimate(egarch(Constant=2,GARCH=0.3,ARCH=2.5,Leverage=-2),y,Display='off');assert(m.Leverage{1}==-2 && abs(m.GARCH{1})<1);[v,li]=infer(m,y);assert(all(v>0));",
    );
}

#[test]
fn tolerance_applies_to_sparse_and_seasonal_root_moduli_and_fixed_parameters() {
    let mut e = Engine::default();
    data(&mut e, &fixture()["cases"][0]);
    let refs = fixture();
    let radius = 1. - refs["root_checks"]["tolerance"].as_f64().unwrap();
    assert!(refs["root_checks"]["too_close"].as_f64().unwrap() > radius);
    assert!(refs["root_checks"]["inside"].as_f64().unwrap() < radius);
    // NumPy roots([1,0,-0.999]) = +/-0.999499874937461; roots of
    // [1,0,-0.997] = +/-0.9984988733093293. At tolerance .001 only
    // the second polynomial satisfies radius <= .999. This also checks that
    // fixed parameters are checked rather than silently relaxed or changed.
    for group in ["AR", "SAR", "MA", "SMA"] {
        let source = format!(
            "estimate(arima(Constant=0,{group}={{0 0.999}},Variance=1),y,Options=struct('ConstraintTolerance',0.001),Display='off');"
        );
        assert!(
            e.execute(&source)
                .unwrap_err()
                .contains("InfeasibleConstraints"),
            "{group}"
        );
        run(
            &mut e,
            &format!(
                "[m,c,l,i]=estimate(arima(Constant=0,{group}={{0 0.997}},Variance=1),y,Options=struct('ConstraintTolerance',0.001),Display='off');assert(isfinite(l) && i.exitflag==1);"
            ),
        );
    }
    assert!(e.execute("estimate(egarch(Constant=0,GARCH={0 0.999},ARCH=0.000001,Leverage=0),y,Options=struct('ConstraintTolerance',0.001),Display='off');").unwrap_err().contains("InfeasibleConstraints"));
    run(
        &mut e,
        "[m,c,l,i]=estimate(egarch(Constant=0,GARCH={0 0.997},ARCH=0.000001,Leverage=0),y,Options=struct('ConstraintTolerance',0.001),Display='off');assert(isfinite(l) && i.exitflag==1);",
    );
}

#[test]
fn undefined_density_probes_near_dof_and_variance_bounds_still_give_finite_covariance() {
    let mut e = Engine::default();
    data(&mut e, &fixture()["cases"][0]);
    // DoF-2 is smaller than the default covariance difference step: the lower
    // probe has no Student t density. It must be rejected/shrunk, not a NaN score.
    for model in [
        "arima(Constant=0,Variance=1,Distribution='t')",
        "garch(Constant=1,Distribution='t')",
        "arima(Constant=0,Variance=garch(Constant=1),Distribution='t')",
    ] {
        run(
            &mut e,
            &format!(
                "[m,c,l,i]=estimate({model},y,DoF0=2.000001,Options=struct('MaxIterations',0),Display='off');assert(isfinite(l) && all(isfinite(c(:))) && i.X(end)>2);"
            ),
        );
    }
}
