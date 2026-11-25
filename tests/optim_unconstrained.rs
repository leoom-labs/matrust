//! fminunc, lsqnonlin, lsqcurvefit, fsolve, optimoptions and the handle forms of
//! fzero, fminbnd and fminsearch.
//!
//! Expected values are analytic optima where known, else SciPy 1.18 from
//! tool/refs/optim/unconstrained.py (tests/fixtures/optim/unconstrained.json).
//! Each tolerance is stated next to its assertion.
use matrust::{Array, Engine};
use std::time::{Duration, Instant};

fn fixture() -> serde_json::Value {
    let text = include_str!("fixtures/optim/unconstrained.json");
    serde_json::from_str(text).unwrap()
}
fn fixture_vec(v: &serde_json::Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn run(e: &mut Engine, code: &str) -> Vec<String> {
    match e.execute(code) {
        Ok(out) => out.output,
        Err(error) => panic!("{code}\n{error}"),
    }
}
fn fails(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(_) => panic!("{code} should fail"),
        Err(error) => error.to_string(),
    }
}
fn values(e: &Engine, name: &str) -> Vec<f64> {
    if let Some(m) = e.variables.get(name) {
        return m.data().to_vec();
    }
    match e.array(name) {
        Some(Array::Numeric(m)) => m.data().to_vec(),
        Some(Array::Logical(m)) => m.data().iter().map(|&b| f64::from(b)).collect(),
        _ => panic!("{name} is not a numeric variable"),
    }
}
fn scalar(e: &Engine, name: &str) -> f64 {
    let v = values(e, name);
    assert_eq!(v.len(), 1, "{name}");
    v[0]
}
fn text(e: &Engine, name: &str) -> String {
    match e.array(name) {
        Some(Array::Character(c)) => c.to_utf8(&e.cancellation()).unwrap(),
        _ => panic!("{name} is not text"),
    }
}
/// |actual - expected| <= tol elementwise.
fn close(actual: &[f64], expected: &[f64], tol: f64, what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}: {actual:?}");
    for (a, b) in actual.iter().zip(expected) {
        assert!(
            (a - b).abs() <= tol,
            "{what}: {actual:?} vs {expected:?} (tol {tol:e})"
        );
    }
}

const ROSEN: &str = "rosen = @(x) 100*(x(2)-x(1)^2)^2 + (1-x(1))^2;";

#[test]
fn fminunc_rosenbrock_reaches_the_minimum() {
    let mut e = Engine::default();
    // Central differences and tight tolerances: x within 1e-6 of [1 1].
    run(
        &mut e,
        &format!(
            "{ROSEN} o = optimoptions('fminunc', 'Display', 'off', 'FiniteDifferenceType', 'central', \
             'OptimalityTolerance', 1e-12, 'StepTolerance', 1e-14, 'FunctionTolerance', 1e-20, 'MaxFunctionEvaluations', 2000); \
             [x, fval, flag, out, grad, hess] = fminunc(rosen, [-1.2 1], o); \
             it = out.iterations; fc = out.funcCount;"
        ),
    );
    close(&values(&e, "x"), &[1., 1.], 1e-6, "Rosenbrock x");
    let sp = fixture_vec(&fixture()["rosenbrock_bfgs"]["x"]);
    close(&values(&e, "x"), &sp, 1e-6, "Rosenbrock vs SciPy BFGS");
    assert!(scalar(&e, "fval") < 1e-12, "fval {}", scalar(&e, "fval"));
    assert!(scalar(&e, "flag") > 0.);
    assert!(scalar(&e, "it") > 10. && scalar(&e, "it") < 200.);
    assert!(scalar(&e, "fc") > scalar(&e, "it"));
    // The shape of x0 (a row) is kept; the gradient is a column.
    assert_eq!(e.variables["x"].rows(), 1);
    assert!(values(&e, "grad").iter().all(|g| g.abs() < 1e-5));
    // The BFGS matrix approximates the Hessian [802 -400; -400 200] at [1 1]: it is
    // symmetric positive definite; its entries within 25 %.
    let h = values(&e, "hess");
    assert_eq!(h.len(), 4);
    assert!((h[1] - h[2]).abs() <= 1e-9 * h[0].abs());
    assert!(h[0] > 0. && h[0] * h[3] - h[1] * h[2] > 0.);
    close(
        &[h[0] / 802., h[1] / -400., h[3] / 200.],
        &[1., 1., 1.],
        0.25,
        "Hessian ratios",
    );

    // Default options (forward differences): within 1e-3, and a final summary line.
    let out = run(&mut e, "[x2, f2, flag2] = fminunc(rosen, [-1.2 1]);");
    close(&values(&e, "x2"), &[1., 1.], 1e-3, "Rosenbrock defaults");
    assert!(scalar(&e, "flag2") > 0.);
    assert!(
        out.iter().any(|l| l.contains("fminunc stopped after")),
        "{out:?}"
    );
}

#[test]
fn fminunc_quadratic_and_user_gradient() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("rosen_grad.m"),
        "function [f, g] = rosen_grad(x)\n\
         f = 100*(x(2)-x(1)^2)^2 + (1-x(1))^2;\n\
         g = [-400*x(1)*(x(2)-x(1)^2) - 2*(1-x(1)); 200*(x(2)-x(1)^2)];\n\
         end\n",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();
    // A strictly convex quadratic with minimum c and value 3: x within 1e-6.
    run(
        &mut e,
        "A = [4 1 0; 1 3 1; 0 1 2]; c = [1; -2; 0.5]; \
         q = @(x) 0.5*(x-c)'*A*(x-c) + 3; \
         [x, fval, flag] = fminunc(q, zeros(3,1), optimoptions('fminunc','Display','off','FunctionTolerance',1e-15));",
    );
    close(&values(&e, "x"), &[1., -2., 0.5], 1e-6, "quadratic x");
    close(&[scalar(&e, "fval")], &[3.], 1e-10, "quadratic fval");
    assert_eq!(scalar(&e, "flag"), 1.);
    // SpecifyObjectiveGradient: the named function's second output is the gradient;
    // optimset's GradObj names the same option.
    run(
        &mut e,
        "o = optimoptions('fminunc','SpecifyObjectiveGradient',true,'Display','off', \
         'OptimalityTolerance',1e-12,'StepTolerance',1e-15,'FunctionTolerance',1e-20); \
         [x, f, flag, out] = fminunc(@rosen_grad, [-1.2; 1], o); fc = out.funcCount; \
         [y, f2] = fminunc('rosen_grad', [-1.2; 1], optimset('GradObj','on','Display','off','TolFun',1e-12,'TolX',1e-15));",
    );
    close(&values(&e, "x"), &[1., 1.], 1e-8, "analytic gradient x");
    close(&values(&e, "y"), &[1., 1.], 1e-8, "optimset GradObj x");
    // One evaluation per trial point when the gradient is supplied.
    assert!(scalar(&e, "fc") < 200., "funcCount {}", scalar(&e, "fc"));
}

#[test]
fn lsqcurvefit_recovers_an_exponential_model() {
    let mut e = Engine::default();
    let reference = fixture();
    run(
        &mut e,
        "t = linspace(0, 4, 25)'; y = 2.5*exp(-1.3*t) + 0.5 + 0.01*sin(7*t); \
         model = @(p, t) p(1)*exp(-p(2)*t) + p(3); \
         o = optimoptions('lsqcurvefit','Display','off','FunctionTolerance',1e-15, \
         'StepTolerance',1e-15,'OptimalityTolerance',1e-10); \
         [p, resnorm, residual, flag, out] = lsqcurvefit(model, [1 1 0], t, y, [], [], o); \
         alg = out.algorithm; \
         [p2, r2] = lsqcurvefit(model, [1 1 0], t, y);",
    );
    let p = values(&e, "p");
    // The ripple moves the fit 0.02 at most from the generating parameters.
    close(&p, &[2.5, 1.3, 0.5], 0.02, "generating parameters");
    // Same least-squares minimum as SciPy least_squares (lm): 1e-7 per parameter.
    close(&p, &fixture_vec(&reference["expfit"]["x"]), 1e-7, "SciPy x");
    let sp_resnorm = reference["expfit"]["resnorm"].as_f64().unwrap();
    close(&[scalar(&e, "resnorm")], &[sp_resnorm], 1e-12, "resnorm");
    let r = values(&e, "residual");
    assert_eq!(r.len(), 25);
    close(
        &[r.iter().map(|v| v * v).sum()],
        &[sp_resnorm],
        1e-14,
        "residual",
    );
    assert_eq!(e.variables["residual"].cols(), 1);
    assert!(scalar(&e, "flag") > 0.);
    // Default tolerances: within 1e-5 of the tight fit.
    close(&values(&e, "p2"), &p, 1e-5, "default options");
}

#[test]
fn lsqnonlin_with_bounds_active() {
    let mut e = Engine::default();
    let reference = fixture();
    // Upper bound of x(1) active: analytic [2 0.2], resnorm 2.8 (1e-9).
    run(
        &mut e,
        "r = @(x) [x(1) - 3; x(2) + 1; x(1)*x(2) - 1]; \
         o = optimoptions('lsqnonlin','Display','off'); \
         [x, resnorm, res, flag, out] = lsqnonlin(r, [1 1], [0 0], [2 2], o); \
         [x0, rn0] = lsqnonlin(r, [1 1]);",
    );
    close(&values(&e, "x"), &[2., 0.2], 1e-9, "bounded x");
    close(
        &values(&e, "x"),
        &fixture_vec(&reference["bounded"]["x"]),
        1e-9,
        "SciPy trf",
    );
    close(&[scalar(&e, "resnorm")], &[2.8], 1e-9, "bounded resnorm");
    assert!(scalar(&e, "flag") > 0.);
    // Without bounds the minimum is lower and x(1) passes the bound.
    assert!(scalar(&e, "rn0") < 2.8 && values(&e, "x0")[0] > 2.);
    // Lower bounds active on a linear residual: [0 2] exactly.
    run(
        &mut e,
        "[z, rn, ~, f2] = lsqnonlin(@(x) x - [-1; 2], [3; 3], [0; 0], [], optimset('Display','off'));",
    );
    close(&values(&e, "z"), &[0., 2.], 1e-12, "lower bound");
    close(&[scalar(&e, "rn")], &[1.], 1e-12, "lower-bound resnorm");
    // Inconsistent bounds are errors naming the offending coordinate.
    let error = fails(&mut e, "lsqnonlin(r, [1 1], [3 0], [2 2]);");
    assert!(
        error.contains("lb(1)") && error.contains("ub(1)"),
        "{error}"
    );
}

#[test]
fn fsolve_finds_roots_of_two_by_two_systems() {
    let mut e = Engine::default();
    let reference = fixture();
    // x1^2 + x2^2 = 5, x1 x2 = 2 has the root [1 2] near the start: 1e-10.
    run(
        &mut e,
        "F = @(x) [x(1)^2 + x(2)^2 - 5; x(1)*x(2) - 2]; \
         [x, fval, flag, out, J] = fsolve(F, [0.5; 3], optimoptions('fsolve','Display','off','OptimalityTolerance',1e-12));",
    );
    close(&values(&e, "x"), &[1., 2.], 1e-10, "root");
    assert!(values(&e, "fval").iter().all(|v| v.abs() < 1e-10));
    assert_eq!(scalar(&e, "flag"), 1.);
    // Forward-difference Jacobian [2 x1, 2 x2; x2, x1] at the root: 1e-6.
    close(&values(&e, "J"), &[2., 2., 4., 1.], 1e-6, "Jacobian");
    // No closed form: SciPy root (hybr) from [0 0], 1e-9.
    run(
        &mut e,
        "G = @(x) [x(1) + 0.5*(x(1)-x(2))^3 - 1; 0.5*(x(2)-x(1))^3 + x(2)]; \
         [y, gv, flag2] = fsolve(G, [0 0], optimoptions('fsolve','Display','off'));",
    );
    close(
        &values(&e, "y"),
        &fixture_vec(&reference["system_hybr"]["x"]),
        1e-9,
        "hybr",
    );
    assert!(scalar(&e, "flag2") > 0.);
    // No real root: x^2 + 1 = 0 ends at a minimum of sum(F.^2) with exitflag -2.
    run(
        &mut e,
        "[z, zv, flag3] = fsolve(@(x) x^2 + 1, 0, optimset('Display','off'));",
    );
    assert_eq!(scalar(&e, "flag3"), -2.);
    close(&values(&e, "z"), &[0.], 1e-3, "minimum of x^2+1");
}

#[test]
fn old_solvers_take_function_handles_with_captured_variables() {
    let mut e = Engine::default();
    // The handle keeps a = 2 although a changes afterwards: sqrt(2) to 1e-12.
    run(
        &mut e,
        "a = 2; f = @(x) x^2 - a; a = 7; r = fzero(f, 1); \
         [r2, fv, flag] = fzero(@(x) x^3 - a, [0 3]); \
         c = fzero(@cos, [1 2]); s = fzero('cos', 1);",
    );
    close(&[scalar(&e, "r")], &[2f64.sqrt()], 1e-12, "fzero anonymous");
    close(&[scalar(&e, "r2")], &[7f64.cbrt()], 1e-12, "fzero bracket");
    assert_eq!(scalar(&e, "flag"), 1.);
    close(
        &[scalar(&e, "c")],
        &[std::f64::consts::FRAC_PI_2],
        1e-12,
        "fzero @cos",
    );
    close(
        &[scalar(&e, "s")],
        &[std::f64::consts::FRAC_PI_2],
        1e-12,
        "fzero text",
    );
    // fminbnd stops within its TolX 1e-4; 1e-4 bound on x.
    run(
        &mut e,
        "b = 1.5; [m, fm, flag] = fminbnd(@(x) (x - b)^2 + 1, 0, 5); m2 = fminbnd(@sin, 3, 5);",
    );
    close(&[scalar(&e, "m")], &[1.5], 1e-4, "fminbnd");
    close(&[scalar(&e, "fm")], &[1.], 1e-8, "fminbnd fval");
    close(
        &[scalar(&e, "m2")],
        &[1.5 * std::f64::consts::PI],
        1e-4,
        "fminbnd @sin",
    );
    // fminsearch: TolX 1e-4, so 1e-3 on x.
    run(
        &mut e,
        "c0 = [1 -2]; [v, fv, flag] = fminsearch(@(x) sum((x - c0).^2), [0 0]);",
    );
    close(&values(&e, "v"), &[1., -2.], 1e-3, "fminsearch");
    assert_eq!(scalar(&e, "flag"), 1.);
    assert!(fails(&mut e, "fzero(3, 1)").contains("function handle"));
}

#[test]
fn options_from_optimoptions_and_optimset() {
    let mut e = Engine::default();
    run(&mut e, ROSEN);
    // Dot assignment on the optimoptions result.
    run(
        &mut e,
        "o = optimoptions('fminunc'); o.MaxIterations = 3; o.Display = 'off'; \
         [x, f, flag, out] = fminunc(rosen, [-1.2 1], o); it = out.iterations;",
    );
    assert_eq!(scalar(&e, "flag"), 0.);
    assert_eq!(scalar(&e, "it"), 3.);
    // optimset names: MaxIter, and MaxFunEvals for lsqnonlin.
    run(
        &mut e,
        "[x, f, flag2] = fminunc(rosen, [-1.2 1], optimset('MaxIter', 2, 'Display', 'off')); \
         [y, rn, res, flag3, out3] = lsqnonlin(@(x) [10*(x(2)-x(1)^2); 1-x(1)], [-1.2 1], [], [], \
         optimset('MaxFunEvals', 5, 'Display', 'off')); fc = out3.funcCount;",
    );
    assert_eq!(scalar(&e, "flag2"), 0.);
    assert_eq!(scalar(&e, "flag3"), 0.);
    assert_eq!(scalar(&e, "fc"), 5.);
    // optimoptions(oldopts, ...) updates; the struct holds the canonical names.
    run(
        &mut e,
        "p = optimoptions('lsqnonlin', 'MaxIter', 9); p = optimoptions(p, 'TolX', 1e-9); \
         mi = p.MaxIterations; st = p.StepTolerance;",
    );
    assert_eq!(scalar(&e, "mi"), 9.);
    assert_eq!(scalar(&e, "st"), 1e-9);
    // Display: 'iter' prints a header and one line per iteration, 'off' nothing.
    let iter = run(
        &mut e,
        "fsolve(@(x) x^3 - 8, 1, optimoptions('fsolve', 'Display', 'iter'));",
    );
    assert!(iter.len() >= 4, "{iter:?}");
    assert!(iter[0].contains("iter"), "{iter:?}");
    let quiet = run(
        &mut e,
        "fsolve(@(x) x^3 - 8, 1, optimoptions('fsolve', 'Display', 'off'));",
    );
    assert!(quiet.is_empty(), "{quiet:?}");
    let final_line = run(&mut e, "fsolve(@(x) x^3 - 8, 1);");
    assert_eq!(final_line.len(), 1, "{final_line:?}");
    // Checked names and values.
    assert!(fails(&mut e, "optimoptions('fminunc', 'MaxIteration', 3)").contains("unknown option"));
    assert!(fails(&mut e, "optimoptions('fminunc', 'Display', 'loud')").contains("Display"));
    assert!(
        fails(&mut e, "optimoptions('fminunc', 'MaxIterations', -1)").contains("MaxIterations")
    );
    assert!(fails(&mut e, "optimoptions('uifigure')").contains("not supported"));
    assert!(fails(&mut e, "optimoptions('fminunc','Algorithm','bogus')").contains("Algorithm"));
    assert!(
        fails(&mut e, "fminunc(rosen, [0 0], struct('Algorithm','bogus'))").contains("Algorithm")
    );
}

#[test]
fn fminunc_unbounded_descent_stops_before_the_budget() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[x, f, flag, out] = fminunc(@(x) -x, 0, optimoptions('fminunc','Display','off','MaxFunctionEvaluations',100)); fc = out.funcCount; step = out.stepsize;",
    );
    assert_eq!(scalar(&e, "flag"), -3.);
    assert!(scalar(&e, "fc") <= 100.);
    assert!(scalar(&e, "f") < -1e20 || scalar(&e, "step") > 1e20);
}

#[test]
fn evaluation_budgets_cover_initial_differences_trials_and_retries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("count_objective.m"),
        "function f = count_objective(x)\ndisp('evaluation');\nf = 100*(x(2)-x(1)^2)^2 + (1-x(1))^2;\nend\n").unwrap();
    std::fs::write(dir.path().join("count_residual.m"),
        "function r = count_residual(x)\ndisp('evaluation');\nr = [10*(x(2)-x(1)^2); 1-x(1)];\nend\n").unwrap();
    for difference in ["forward", "central"] {
        for limit in [0, 1, 2, 3, 4, 5, 6, 10, 18] {
            for solver in ["fminunc", "lsqnonlin", "lsqcurvefit", "fsolve"] {
                let mut e = Engine::default();
                e.set_function_directory(dir.path()).unwrap();
                let args = match solver {
                    "fminunc" => "@count_objective, [-1.2 1], o",
                    "lsqnonlin" => "@count_residual, [-1.2 1], [], [], o",
                    "lsqcurvefit" => "@(x,t) count_residual(x), [-1.2 1], 0, [0; 0], [], [], o",
                    _ => "@count_residual, [-1.2 1], o",
                };
                let outputs = if solver.starts_with("lsq") {
                    "x, f, r, flag, out"
                } else {
                    "x, f, flag, out"
                };
                let printed = run(
                    &mut e,
                    &format!(
                        "o = optimoptions('{solver}','Display','off','MaxFunctionEvaluations',{limit},'FiniteDifferenceType','{difference}'); [{outputs}] = {solver}({args}); fc = out.funcCount;"
                    ),
                );
                assert_eq!(scalar(&e, "flag"), 0., "{solver}/{difference}/{limit}");
                assert_eq!(
                    scalar(&e, "fc"),
                    limit as f64,
                    "{solver}/{difference}/{limit}"
                );
                assert_eq!(
                    printed.len(),
                    limit,
                    "callback count: {solver}/{difference}/{limit}"
                );
                if solver == "fsolve" && limit == 0 {
                    run(&mut e, "alg = out.algorithm;");
                    assert_eq!(text(&e, "alg"), "trust-region-dogleg");
                }
            }
        }
    }
}

#[test]
fn failed_wolfe_search_uses_scaled_armijo_and_reports_failure() {
    let dir = tempfile::tempdir().unwrap();
    // Deliberately inconsistent derivative exercises rejection of every trial.
    std::fs::write(
        dir.path().join("flat_gradient.m"),
        "function [f,g] = flat_gradient(x)\ndisp('evaluation');\nf = 1; g = 2;\nend\n",
    )
    .unwrap();
    // A tiny linear objective cannot meet Wolfe curvature in 30 trials, but
    // Armijo accepts -g. The analytic optimum is unbounded; stop after one step.
    std::fs::write(
        dir.path().join("tiny_linear.m"),
        "function [f,g] = tiny_linear(x)\nf = -1e-30*x; g = -1e-30;\nend\n",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();
    let printed = run(
        &mut e,
        "o = optimoptions('fminunc','Display','off','SpecifyObjectiveGradient',true,'MaxFunctionEvaluations',200); \
         [x,f,flag,out,g,h] = fminunc(@flat_gradient,0,o); fc = out.funcCount;",
    );
    assert_eq!(scalar(&e, "flag"), -3.);
    // One initial call plus <=30 Wolfe and <=30 Armijo calls.
    assert!(scalar(&e, "fc") <= 61.);
    assert_eq!(printed.len(), scalar(&e, "fc") as usize);
    assert_eq!(scalar(&e, "h"), 2., "B resets to a scaled identity");
    run(
        &mut e,
        "o.OptimalityTolerance = 1e-100; o.FunctionTolerance = 1e-100; o.StepTolerance = 1e-100; o.MaxIterations = 1; \
         [x,f,flag,out] = fminunc(@tiny_linear,0,o); fc = out.funcCount;",
    );
    assert_eq!(scalar(&e, "flag"), 0.);
    assert_eq!(
        scalar(&e, "fc"),
        32.,
        "30 Wolfe trials followed by one Armijo trial"
    );
    close(&[scalar(&e, "x")], &[1e-30], 1e-44, "Armijo step along -g");
}

#[test]
fn rejected_trust_regions_return_minus_three() {
    let dir = tempfile::tempdir().unwrap();
    // Constant residual and inconsistent Jacobian: no proposed step can improve it.
    std::fs::write(
        dir.path().join("flat_jacobian.m"),
        "function [r,j] = flat_jacobian(x)\nr = 1; j = 1;\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("flat_curve.m"),
        "function [r,j] = flat_curve(x,t)\nr = 1; j = 1;\nend\n",
    )
    .unwrap();
    for solver in ["lsqnonlin", "lsqcurvefit", "fsolve"] {
        let mut e = Engine::default();
        e.set_function_directory(dir.path()).unwrap();
        let (outputs, args) = match solver {
            "lsqnonlin" => ("x,rn,r,flag,out", "@flat_jacobian,0,[],[],o"),
            "lsqcurvefit" => ("x,rn,r,flag,out", "@flat_curve,0,0,0,[],[],o"),
            _ => ("x,r,flag,out", "@flat_jacobian,0,o"),
        };
        run(
            &mut e,
            &format!(
                "o = optimoptions('{solver}','Display','off','SpecifyObjectiveGradient',true); [{outputs}] = {solver}({args});"
            ),
        );
        assert_eq!(scalar(&e, "flag"), -3., "{solver}");
        assert_eq!(scalar(&e, "x"), 0.);
    }
}

#[test]
fn algorithm_selection_uses_lm_for_rectangular_systems() {
    let mut e = Engine::default();
    for (f, x0, expected) in [
        ("@(x) [x-2; 2*(x-2)]", "0", vec![2.]),
        ("@(x) x(1)+x(2)-2", "[0 0]", vec![1., 1.]),
        ("@(x) x^2-4", "3", vec![2.]),
    ] {
        for algorithm in ["levenberg-marquardt", "trust-region-dogleg"] {
            run(
                &mut e,
                &format!(
                    "o = optimoptions('fsolve','Display','off','Algorithm','{algorithm}','OptimalityTolerance',1e-12); \
                [x,r,flag,out] = fsolve({f},{x0},o); alg = strcmp(out.algorithm,'levenberg-marquardt');"
                ),
            );
            close(&values(&e, "x"), &expected, 1e-8, "analytic LM root");
            assert_eq!(scalar(&e, "flag"), 1.);
            assert_eq!(
                scalar(&e, "alg"),
                f64::from(algorithm == "levenberg-marquardt" || f != "@(x) x^2-4")
            );
        }
    }
    run(
        &mut e,
        "o = optimoptions('lsqnonlin','Algorithm','trust-region-reflective');",
    );
    assert!(fails(&mut e, "lsqcurvefit(@(x,t) x*t, 1, 1, 0, [], [], o)").contains("Algorithm"));
}

#[test]
fn bounds_errors_name_the_bound_and_projection_is_reported() {
    let mut e = Engine::default();
    for solver in ["lsqnonlin", "lsqcurvefit"] {
        let fun = if solver == "lsqnonlin" {
            "@(x) x-1, 5"
        } else {
            "@(x,t) x*t, 5, 1, 1"
        };
        assert!(fails(&mut e, &format!("{solver}({fun},2,1)")).contains("lb(1)"));
        run(
            &mut e,
            &format!(
                "[x,rn,r,flag,out] = {solver}({fun},0,2,optimset('Display','off','MaxIter',0)); message = out.message;"
            ),
        );
        assert_eq!(scalar(&e, "x"), 2.);
        assert!(text(&e, "message").contains("projected"));
        assert_eq!(scalar(&e, "rn"), 1.);
    }
}

#[test]
fn tolerances_must_be_positive_in_builders_and_solver_structs() {
    let mut e = Engine::default();
    for name in [
        "OptimalityTolerance",
        "StepTolerance",
        "FunctionTolerance",
        "TolX",
        "TolFun",
    ] {
        for value in ["0", "-1"] {
            let error = fails(&mut e, &format!("optimoptions('fminunc','{name}',{value})"));
            let canonical = if name == "TolX" {
                "StepTolerance"
            } else {
                name
            };
            assert!(error.contains(canonical), "{error}");
            for call in [
                "fminunc(@(x) x^2,1,o)",
                "fsolve(@(x) x,1,o)",
                "lsqnonlin(@(x) x,1,[],[],o)",
                "lsqcurvefit(@(x,t) x*t,1,1,0,[],[],o)",
            ] {
                run(
                    &mut e,
                    &format!("o = optimoptions('fminunc'); o.{name} = {value};"),
                );
                let error = fails(&mut e, call);
                assert!(error.contains(canonical), "{error}");
            }
        }
    }
}

#[test]
fn unsupported_nonempty_struct_options_are_errors() {
    let mut e = Engine::default();
    for call in [
        "fminunc(@(x) x^2,1,o)",
        "fsolve(@(x) x,1,o)",
        "lsqnonlin(@(x) x,1,[],[],o)",
        "lsqcurvefit(@(x,t) x*t,1,1,0,[],[],o)",
    ] {
        run(&mut e, "o = optimset('PlotFcns',1);");
        assert!(fails(&mut e, call).contains("PlotFcns"));
        run(&mut e, "o = optimoptions('fminunc'); o.OutputFcn = 1;");
        assert!(fails(&mut e, call).contains("OutputFcn"));
        run(&mut e, "o = optimset('PlotFcns',[],'Display','off');");
        run(&mut e, call);
    }
}

#[test]
fn constrained_only_options_are_rejected_by_nonlinear_solvers() {
    let mut e = Engine::default();
    for (solver, args, outputs) in [
        ("fminunc", "@(x)x^2,1", "x,f,flag,out"),
        ("fsolve", "@(x)x,1", "x,f,flag,out"),
        ("lsqnonlin", "@(x)x,1,[],[]", "x,f,residual,flag,out"),
        (
            "lsqcurvefit",
            "@(x,t)x*t,1,1,0,[],[]",
            "x,f,residual,flag,out",
        ),
    ] {
        for (field, value) in [
            ("ConstraintTolerance", "1e-6"),
            ("TolCon", "1e-6"),
            ("MaxNodes", "1"),
            ("IntegerTolerance", "1e-6"),
            ("RelativeGapTolerance", "0"),
            ("AbsoluteGapTolerance", "0"),
            ("SpecifyConstraintGradient", "false"),
            ("GradConstr", "false"),
        ] {
            for option in [
                format!("struct('{field}',{value})"),
                format!("optimoptions('{solver}','{field}',{value})"),
            ] {
                let source = format!("{solver}({args},{option})");
                let err = fails(&mut e, &source);
                let canonical = match field {
                    "TolCon" => "ConstraintTolerance",
                    "GradConstr" => "SpecifyConstraintGradient",
                    _ => field,
                };
                assert!(
                    err.contains(field) || err.contains(canonical),
                    "{source}: {err}"
                );
                assert!(err.contains("not implemented"), "{source}: {err}");
            }
            let builder = format!("optimoptions('{solver}','{field}',{value})");
            assert!(fails(&mut e, &builder).contains("not implemented"));
            // Empty unsupported fields keep defaults; shared supported fields
            // still apply in both raw structures and optimoptions construction.
            for option in [
                format!("struct('{field}',[],'Display','off','MaxIterations',0)"),
                format!("optimoptions('{solver}','{field}',[],'Display','off','MaxIterations',0)"),
            ] {
                run(
                    &mut e,
                    &format!("[{outputs}]={solver}({args},{option}); it=out.iterations;"),
                );
                assert_eq!(scalar(&e, "it"), 0.);
                assert_eq!(scalar(&e, "x"), 1.);
            }
        }
    }
}

#[test]
fn fsolve_optimality_is_relative_to_the_initial_gradient() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[x,r,flag,out] = fsolve(@(x) 1000*(x^2-1),2,optimset('Display','off')); opt = out.firstorderopt;",
    );
    // Analytic J'F at x0=2 is 4000*3000=12e6.
    assert_eq!(scalar(&e, "flag"), 1.);
    assert!(scalar(&e, "opt") > 1e-6 && scalar(&e, "opt") <= 12.);
    assert!(
        scalar(&e, "r").powi(2) <= 1e-3,
        "retain the existing root rule"
    );
}

#[test]
fn fminunc_function_tolerance_can_stop_with_flag_three() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[x,f,flag,out] = fminunc(@(x) 1e6+(x-3)^2,0,optimoptions('fminunc','Display','off','FunctionTolerance',1e-4)); it = out.iterations;",
    );
    assert_eq!(scalar(&e, "flag"), 3.);
    assert_eq!(scalar(&e, "it"), 1.);
    // The first step is x=1; |f1-f0|/max(1,|f0|) = 5/1000009 < 1e-4.
    close(
        &[scalar(&e, "x")],
        &[1.],
        1e-12,
        "relative function stopping point",
    );
}

#[test]
fn least_squares_default_evaluation_budget_is_one_hundred_per_unknown() {
    let dir = tempfile::tempdir().unwrap();
    // No trial can improve this deliberately inconsistent residual/Jacobian.
    // Tiny tolerances keep rejection retries running until the default budget.
    std::fs::write(
        dir.path().join("budget_residual.m"),
        "function [r,j] = budget_residual(x)\ndisp('evaluation');\nr = 1; j = 1;\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("budget_curve.m"),
        "function [r,j] = budget_curve(x,t)\ndisp('evaluation');\nr = 1; j = 1;\nend\n",
    )
    .unwrap();
    for solver in ["lsqnonlin", "lsqcurvefit"] {
        let mut e = Engine::default();
        e.set_function_directory(dir.path()).unwrap();
        let args = if solver == "lsqnonlin" {
            "@budget_residual,0,[],[],o"
        } else {
            "@budget_curve,0,0,0,[],[],o"
        };
        let printed = run(
            &mut e,
            &format!(
                "o = optimoptions('{solver}','Display','off','SpecifyObjectiveGradient',true,'StepTolerance',1e-300,'FunctionTolerance',1e-300,'OptimalityTolerance',1e-300); \
             [x,rn,r,flag,out] = {solver}({args}); fc = out.funcCount;"
            ),
        );
        assert_eq!(scalar(&e, "flag"), 0., "{solver}");
        assert_eq!(scalar(&e, "fc"), 100., "{solver}");
        assert_eq!(printed.len(), 100, "{solver} callback count");
    }
}

#[test]
fn cancel_stops_a_never_converging_run() {
    let mut e = Engine::default();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("slow_residual.m"),
        "function r = slow_residual(x)\nk = 0;\nwhile k < 1000000\nr = [x(1)-3+1e-3*rand(); x(2)*rand()]; k = k+1;\nend\nend\n").unwrap();
    e.set_function_directory(dir.path()).unwrap();
    // Tiny positive tolerances with no iteration/evaluation limit. A costly
    // noisy callback keeps this deterministic: cancellation precedes convergence.
    let code = "o = optimoptions('lsqnonlin', 'Display', 'off', 'MaxIterations', Inf, \
                'MaxFunctionEvaluations', Inf, 'OptimalityTolerance', 1e-300, 'StepTolerance', 1e-300, \
                'FunctionTolerance', 1e-300); \
                x = lsqnonlin(@slow_residual, [0 1], [], [], o);";
    let cancel = e.cancellation();
    let setter = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let start = Instant::now();
    let error = e.execute(code).expect_err("the run must be cancelled");
    let elapsed = start.elapsed();
    setter.join().unwrap();
    assert!(error.to_lowercase().contains("cancel"), "{error}");
    // Cancelled promptly after the flag was set (generous bound for a loaded machine).
    assert!(
        elapsed >= Duration::from_millis(300) && elapsed < Duration::from_secs(20),
        "{elapsed:?}"
    );
}

#[test]
fn changing_result_length_is_an_error_not_a_panic() {
    let mut e = Engine::default();
    for code in [
        "x = lsqnonlin(@(x) ones(1+(x<0.5),1).*(x-1), 0);",
        "x = fsolve(@(x) ones(1+(x<0.5),1).*(x-1), 0);",
        "x = fminunc(@(x) x(x<=0).^2+1, 0);",
    ] {
        let message = fails(&mut e, code);
        assert!(
            message.contains("result length must not change")
                || message.contains("must return a scalar"),
            "{code}: {message}"
        );
    }
}

#[test]
fn fminunc_trust_region_hessian_options_warn_and_run_bfgs_with_gradient() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("convex_fgh.m"),
        "function [f, g, H] = convex_fgh(x)\n\
         f = 0.5 * ((x(1) - 2)^2 + 4 * (x(2) + 1)^2);\n\
         if nargout > 1\n\
             g = [x(1) - 2; 4 * (x(2) + 1)];\n\
         end\n\
         if nargout > 2\n\
             H = [1, 0; 0, 4];\n\
         end\n\
         end\n",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();

    // Constructor warns once per accepted unimplemented option (Algorithm, HessianFcn),
    // mentions fminunc and the option name, and does not warn for SpecifyObjectiveGradient.
    let warnings = run(
        &mut e,
        "o = optimoptions('fminunc','Algorithm','trust-region','HessianFcn','objective','SpecifyObjectiveGradient',true,'HessianFcn','objective');",
    );
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings[0].contains("fminunc") && warnings[0].contains("Algorithm"),
        "{warnings:?}"
    );
    assert!(
        warnings[1].contains("fminunc") && warnings[1].contains("HessianFcn"),
        "{warnings:?}"
    );

    // Solve with the resulting options (Display off so only solver warnings would appear).
    let solve_out = run(
        &mut e,
        "o2 = optimoptions(o, 'Display', 'off'); \
         [x, fval, flag, out, g, H] = fminunc(@convex_fgh, [0; 0], o2); \
         alg = out.algorithm; fc = out.funcCount;",
    );
    assert!(
        solve_out.is_empty(),
        "no extra output/warnings expected: {solve_out:?}"
    );
    close(&values(&e, "x"), &[2., -1.], 1e-4, "convex fminunc x");
    assert!(scalar(&e, "fval") < 1e-8);
    let flag = scalar(&e, "flag");
    assert!(flag == 1. || flag == 3., "unexpected exitflag {flag}");
    close(&values(&e, "g"), &[0., 0.], 1e-4, "convex fminunc g");
    let alg = text(&e, "alg");
    assert!(alg.contains("quasi-newton"), "{alg}");
    assert!(
        !alg.to_ascii_lowercase().contains("trust-region")
            && !alg.to_ascii_lowercase().contains("hessian"),
        "{alg}"
    );
    // Analytic gradient takes far fewer evaluations than finite differences on 2 variables.
    assert!(scalar(&e, "fc") <= 10., "fc = {}", scalar(&e, "fc"));
}

#[test]
fn fminunc_r2024b_options_validation_table() {
    let mut e = Engine::default();

    // Honored options (case-insensitive spellings) produce no warnings.
    let honored = run(
        &mut e,
        "o = optimoptions('fminunc','display','off','maxiterations',5,'maxfunctionevaluations',20,\
         'optimalitytolerance',1e-6,'steptolerance',1e-6,'functiontolerance',1e-6,\
         'specifyobjectivegradient',false,'finitedifferencetype','forward','algorithm','quasi-newton');",
    );
    assert!(honored.is_empty(), "{honored:?}");

    // Documented R2024b unimplemented fminunc options: case-insensitive, warn once, pass through to solve.
    for (given, canonical, value) in [
        ("algorithm", "Algorithm", "'trust-region'"),
        ("checkgradients", "CheckGradients", "true"),
        ("diagnostics", "Diagnostics", "'on'"),
        (
            "finitedifferencestepsize",
            "FiniteDifferenceStepSize",
            "1e-6",
        ),
        ("hessianapproximation", "HessianApproximation", "'lbfgs'"),
        ("hessianfcn", "HessianFcn", "'objective'"),
        ("hessianmultiplyfcn", "HessianMultiplyFcn", "@(H,y) y"),
        ("objectivelimit", "ObjectiveLimit", "-1e10"),
        (
            "subproblemalgorithm",
            "SubproblemAlgorithm",
            "'factorization'",
        ),
        ("typicalx", "TypicalX", "[1; 1]"),
        ("useparallel", "UseParallel", "true"),
    ] {
        let out = run(
            &mut e,
            &format!("o = optimoptions('fminunc','Display','off','{given}',{value});"),
        );
        assert_eq!(out.len(), 1, "{given}: {out:?}");
        assert!(
            out[0].contains("fminunc") && out[0].contains(canonical),
            "{given}: {out:?}"
        );
        run(
            &mut e,
            "[x, f, flag] = fminunc(@(x) (x(1)-1)^2 + (x(2)+2)^2, [0; 0], o);",
        );
        assert_eq!(scalar(&e, "flag"), 1., "{given}");
        close(&values(&e, "x"), &[1., -2.], 1e-5, given);
    }

    // Invalid values, typos, and wrong-solver / exclusive options remain hard errors.
    for (call, expected_substr) in [
        (
            "optimoptions('fminunc','Algorithm','levenberg-marquardt')",
            "Algorithm",
        ),
        ("optimoptions('fminunc','Algorithm','bogus')", "Algorithm"),
        ("optimoptions('fminunc','HessianFcn','bfgs')", "HessianFcn"),
        (
            "optimoptions('fminunc','HessianApproximation','sr1')",
            "HessianApproximation",
        ),
        (
            "optimoptions('fminunc','SubproblemAlgorithm','newton')",
            "SubproblemAlgorithm",
        ),
        (
            "optimoptions('fminunc','Diagnostics','loud')",
            "Diagnostics",
        ),
        (
            "optimoptions('fminunc','FiniteDifferenceStepSize',-1)",
            "FiniteDifferenceStepSize",
        ),
        (
            "optimoptions('fminunc','FiniteDifferenceStepSize',0)",
            "FiniteDifferenceStepSize",
        ),
        (
            "optimoptions('fminunc','FiniteDifferenceStepSize',NaN)",
            "FiniteDifferenceStepSize",
        ),
        (
            "optimoptions('fminunc','FiniteDifferenceStepSize',Inf)",
            "FiniteDifferenceStepSize",
        ),
        (
            "optimoptions('fminunc','FiniteDifferenceStepSize',[1e-6 2e-6; 3e-6 4e-6])",
            "FiniteDifferenceStepSize",
        ),
        (
            "optimoptions('fminunc','ObjectiveLimit',NaN)",
            "ObjectiveLimit",
        ),
        ("optimoptions('fminunc','TypicalX',Inf)", "TypicalX"),
        (
            "optimoptions('fminunc','CheckGradients','maybe')",
            "CheckGradients",
        ),
        (
            "optimoptions('fminunc','CheckGradients',NaN)",
            "CheckGradients",
        ),
        (
            "optimoptions('fminunc','CheckGradients',Inf)",
            "CheckGradients",
        ),
        ("optimoptions('fminunc','UseParallel',[1 2])", "UseParallel"),
        ("optimoptions('fminunc','UseParallel',NaN)", "UseParallel"),
        ("optimoptions('fminunc','UseParallel',Inf)", "UseParallel"),
        ("optimoptions('fminunc','UseParallel',-Inf)", "UseParallel"),
        (
            "optimoptions('fminunc','HessianMultiplyFcn',123)",
            "HessianMultiplyFcn",
        ),
        (
            "optimoptions('fminunc','HessianFnc','objective')",
            "unknown option",
        ),
        ("optimoptions('fminunc','SwarmSize',10)", "unknown option"),
        (
            "optimoptions('fminunc','ConstraintTolerance',1e-6)",
            "not implemented",
        ),
        (
            "optimoptions('fsolve','HessianFcn','objective')",
            "unknown option",
        ),
        (
            "optimoptions('lsqnonlin','HessianApproximation','bfgs')",
            "unknown option",
        ),
        (
            "optimoptions('lsqcurvefit','CheckGradients',true)",
            "unknown option",
        ),
        (
            "optimoptions('fzero','HessianFcn','objective')",
            "unknown option",
        ),
    ] {
        let err = fails(&mut e, call);
        assert!(err.contains(expected_substr), "{call}: {err}");
    }
}

#[test]
fn fminunc_option_identity_step_and_bool_checks() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("quad_grad.m"),
        "function [f, g] = quad_grad(x)\n\
         f = (x(1)-1)^2 + (x(2)+2)^2;\n\
         if nargout > 1\n\
             g = [2*(x(1)-1); 2*(x(2)+2)];\n\
         end\n\
         end\n",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();

    // Named fsolve options have no solver tag. A new fminunc-only field must
    // be rejected here, with no fminunc warning and no later fsolve failure.
    run(
        &mut e,
        "fs = optimoptions('fsolve','Display','off'); lastwarn('sentinel','sentinel:id');",
    );
    let err = fails(&mut e, "fs2 = optimoptions(fs,'HessianFcn','objective');");
    assert!(err.contains("HessianFcn"), "{err}");
    assert!(err.contains("does not name a solver"), "{err}");
    assert!(!err.contains("will be ignored"), "{err}");
    assert!(!err.contains("fminunc option"), "{err}");
    run(&mut e, "[m,id] = lastwarn;");
    assert_eq!(text(&e, "m"), "sentinel");
    assert_eq!(text(&e, "id"), "sentinel:id");
    run(&mut e, "[x,fv,flag] = fsolve(@(x) x-1, 0, fs);");
    assert_eq!(scalar(&e, "flag"), 1.);
    close(
        &[scalar(&e, "x")],
        &[1.],
        1e-12,
        "fsolve after rejected HessianFcn",
    );

    // Generic updates still apply to that fsolve structure.
    let generic = run(
        &mut e,
        "fs2 = optimoptions(fs,'Display','off','MaxIterations',25,'MaxFunctionEvaluations',80,'Algorithm','levenberg-marquardt');",
    );
    assert!(generic.is_empty(), "{generic:?}");
    let dogleg = run(
        &mut e,
        "fs3 = optimoptions(fs,'Algorithm','trust-region-dogleg');",
    );
    assert!(dogleg.is_empty(), "{dogleg:?}");
    run(&mut e, "[x,fv,flag] = fsolve(@(x) x.^2-4, 3, fs2);");
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&[scalar(&e, "x")], &[2.], 1e-8, "fsolve generic update");

    // trust-region on an untagged structure is ambiguous: diagnose, do not name fminunc.
    let ambiguous = run(
        &mut e,
        "lastwarn('',''); fs4 = optimoptions(fs,'Algorithm','trust-region'); [m,id] = lastwarn;",
    );
    assert_eq!(ambiguous.len(), 1, "{ambiguous:?}");
    assert!(
        ambiguous[0].contains("Algorithm") && ambiguous[0].contains("trust-region"),
        "{ambiguous:?}"
    );
    assert!(!ambiguous[0].contains("fminunc option"), "{ambiguous:?}");
    assert_eq!(text(&e, "id"), "optimoptions:Algorithm");
    let solve = run(
        &mut e,
        "[x,~,flag,out] = fsolve(@(x) x-1, 0, fs4); used = out.algorithm;",
    );
    assert!(solve.is_empty(), "{solve:?}");
    assert_eq!(text(&e, "used"), "levenberg-marquardt");
    close(&values(&e, "x"), &[1.], 1e-12, "updated fsolve root");
    assert_eq!(scalar(&e, "flag"), 1.);

    // Plain struct: new solver-specific setting rejected; shared fields accepted.
    run(&mut e, "lastwarn('sentinel','sentinel:id');");
    let plain = fails(
        &mut e,
        "optimoptions(struct('Display','off'),'CheckGradients',true);",
    );
    assert!(
        plain.contains("CheckGradients") && plain.contains("does not name a solver"),
        "{plain}"
    );
    run(&mut e, "[m,id] = lastwarn;");
    assert_eq!(text(&e, "m"), "sentinel");
    let plain_ok = run(
        &mut e,
        "p = optimoptions(struct(),'Display','off','MaxIterations',4);",
    );
    assert!(plain_ok.is_empty(), "{plain_ok:?}");

    // Named fminunc options, then Algorithm trust-region: fallback is not silent.
    let base = run(
        &mut e,
        "base = optimoptions('fminunc','Display','off','SpecifyObjectiveGradient',true); n1 = numel(fieldnames(base));",
    );
    assert!(base.is_empty(), "{base:?}");
    let updated = run(
        &mut e,
        "lastwarn('',''); o = optimoptions(base,'Algorithm','trust-region'); \
         [m,id] = lastwarn; named = isfield(o,'Solver'); n2 = numel(fieldnames(o)); alg = o.Algorithm;",
    );
    assert_eq!(updated.len(), 1, "{updated:?}");
    assert!(
        updated[0].contains("Algorithm") && updated[0].contains("quasi-newton"),
        "{updated:?}"
    );
    assert!(!updated[0].contains("fminunc option"), "{updated:?}");
    assert_eq!(text(&e, "id"), "optimoptions:Algorithm");
    assert_eq!(text(&e, "alg"), "trust-region");
    assert_eq!(scalar(&e, "named"), 0.);
    assert_eq!(scalar(&e, "n2"), scalar(&e, "n1") + 1.);
    let solve = run(
        &mut e,
        "[x,f,flag,out] = fminunc(@quad_grad, [0; 0], o); used = out.algorithm; fc = out.funcCount;",
    );
    assert!(solve.is_empty(), "{solve:?}");
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[1., -2.], 1e-8, "updated fminunc x");
    let used = text(&e, "used");
    assert!(used.contains("quasi-newton"), "{used}");
    assert!(
        !used.to_ascii_lowercase().contains("trust-region"),
        "{used}"
    );
    assert!(scalar(&e, "fc") <= 15., "fc = {}", scalar(&e, "fc"));

    // Named construction already warned; a later update must not warn again.
    let named = run(
        &mut e,
        "prev = optimoptions('fminunc','Algorithm','trust-region','Display','off');",
    );
    assert_eq!(named.len(), 1, "{named:?}");
    assert!(
        named[0].contains("fminunc option") && named[0].contains("Algorithm"),
        "{named:?}"
    );
    let again = run(
        &mut e,
        "prev2 = optimoptions(prev,'Algorithm','trust-region','MaxIterations',40);",
    );
    assert!(again.is_empty(), "{again:?}");
    let display = run(
        &mut e,
        "prev3 = optimoptions(prev,'Display','final','MaxFunctionEvaluations',100);",
    );
    assert!(display.is_empty(), "{display:?}");
    let quasi = run(
        &mut e,
        "prev4 = optimoptions(prev,'Algorithm','quasi-newton');",
    );
    assert!(quasi.is_empty(), "{quasi:?}");

    // Copy already-validated fminunc fields, including HessianFcn, without a second warning.
    let copied_from = run(
        &mut e,
        "src = optimoptions('fminunc','Display','off','HessianFcn','objective');",
    );
    assert_eq!(copied_from.len(), 1, "{copied_from:?}");
    let copied = run(
        &mut e,
        "dst = optimoptions(src,'MaxIterations',30,'MaxFunctionEvaluations',60); h = dst.HessianFcn;",
    );
    assert!(copied.is_empty(), "{copied:?}");
    assert_eq!(text(&e, "h"), "objective");
    run(
        &mut e,
        "[x,f,flag] = fminunc(@(x) (x(1)-1)^2 + (x(2)+2)^2, [0; 0], dst);",
    );
    assert_eq!(scalar(&e, "flag"), 1.);
    let reset = run(&mut e, "dst2 = optimoptions(dst,'HessianFcn','objective');");
    assert!(reset.is_empty(), "{reset:?}");
    // A new fminunc-only field on that structure is still unnamed, so it is rejected.
    run(&mut e, "lastwarn('sentinel','sentinel:id');");
    let added = fails(&mut e, "optimoptions(src,'CheckGradients',true);");
    assert!(
        added.contains("CheckGradients") && added.contains("does not name a solver"),
        "{added}"
    );
    run(&mut e, "[m,id] = lastwarn;");
    assert_eq!(text(&e, "m"), "sentinel");
    let bad_existing = fails(&mut e, "optimoptions(dst,'HessianFcn','bogus');");
    assert!(bad_existing.contains("HessianFcn"), "{bad_existing}");
    run(&mut e, "[m,id] = lastwarn;");
    assert_eq!(text(&e, "m"), "sentinel", "rejected value must not warn");

    // FiniteDifferenceStepSize: scalar, row, and column warn once; a 2x2 does not.
    for (shape, value) in [
        ("scalar", "1e-6"),
        ("row", "[1e-8 2e-8]"),
        ("column", "[1e-8; 2e-8]"),
    ] {
        let out = run(
            &mut e,
            &format!(
                "lastwarn('',''); o = optimoptions('fminunc','FiniteDifferenceStepSize',{value}); [m,id] = lastwarn;"
            ),
        );
        assert_eq!(out.len(), 1, "{shape}: {out:?}");
        assert!(
            out[0].contains("FiniteDifferenceStepSize"),
            "{shape}: {out:?}"
        );
        assert_eq!(
            text(&e, "id"),
            "optimoptions:fminunc:FiniteDifferenceStepSize",
            "{shape}"
        );
        assert!(
            text(&e, "m").contains("FiniteDifferenceStepSize"),
            "{shape}"
        );
    }
    for value in ["[1e-6 2e-6; 3e-6 4e-6]", "NaN", "Inf", "0", "[1e-6 NaN]"] {
        run(&mut e, "lastwarn('sentinel','sentinel:id');");
        let err = fails(
            &mut e,
            &format!("optimoptions('fminunc','FiniteDifferenceStepSize',{value});"),
        );
        assert!(err.contains("FiniteDifferenceStepSize"), "{value}: {err}");
        run(&mut e, "[m,id] = lastwarn;");
        assert_eq!(text(&e, "m"), "sentinel", "{value}");
        assert_eq!(text(&e, "id"), "sentinel:id", "{value}");
    }

    // UseParallel and CheckGradients: finite booleans warn once; NaN/Inf do not.
    for (name, value) in [
        ("UseParallel", "true"),
        ("UseParallel", "false"),
        ("UseParallel", "0"),
        ("UseParallel", "1"),
        ("UseParallel", "'on'"),
        ("UseParallel", "'off'"),
        ("CheckGradients", "true"),
        ("CheckGradients", "false"),
        ("CheckGradients", "0"),
        ("CheckGradients", "1"),
        ("CheckGradients", "'on'"),
        ("CheckGradients", "'off'"),
    ] {
        let out = run(
            &mut e,
            &format!(
                "lastwarn('',''); o = optimoptions('fminunc','{name}',{value}); [m,id] = lastwarn;"
            ),
        );
        assert_eq!(out.len(), 1, "{name}={value}: {out:?}");
        assert!(out[0].contains(name), "{name}={value}: {out:?}");
        assert_eq!(
            text(&e, "id"),
            format!("optimoptions:fminunc:{name}"),
            "{name}={value}"
        );
    }
    for (name, value) in [
        ("UseParallel", "NaN"),
        ("UseParallel", "Inf"),
        ("UseParallel", "-Inf"),
        ("UseParallel", "2"),
        ("CheckGradients", "NaN"),
        ("CheckGradients", "Inf"),
        ("CheckGradients", "-Inf"),
        ("CheckGradients", "2"),
    ] {
        run(&mut e, "lastwarn('sentinel','sentinel:id');");
        let err = fails(
            &mut e,
            &format!("optimoptions('fminunc','{name}',{value});"),
        );
        assert!(err.contains(name), "{name}={value}: {err}");
        run(&mut e, "[m,id] = lastwarn;");
        assert_eq!(text(&e, "m"), "sentinel", "{name}={value}");
    }
    // A valid option paired with a rejected one emits no warning.
    run(&mut e, "lastwarn('sentinel','sentinel:id');");
    let pair = fails(
        &mut e,
        "optimoptions('fminunc','FiniteDifferenceStepSize',[1e-6 2e-6],'UseParallel',NaN);",
    );
    assert!(pair.contains("UseParallel"), "{pair}");
    run(&mut e, "[m,id] = lastwarn;");
    assert_eq!(text(&e, "m"), "sentinel", "{pair}");

    // One warning for a repeated valid setting; suppression still records lastwarn.
    let once = run(
        &mut e,
        "o = optimoptions('fminunc','UseParallel',true,'UseParallel',false);",
    );
    assert_eq!(once.len(), 1, "{once:?}");
    let hidden = run(
        &mut e,
        "warning('off','optimoptions:fminunc:CheckGradients'); lastwarn('',''); \
         o = optimoptions('fminunc','CheckGradients',true); [m,id] = lastwarn;",
    );
    assert!(hidden.is_empty(), "{hidden:?}");
    assert!(
        text(&e, "m").contains("CheckGradients"),
        "{}",
        text(&e, "m")
    );
    assert_eq!(text(&e, "id"), "optimoptions:fminunc:CheckGradients");
    let still = run(&mut e, "o = optimoptions('fminunc','UseParallel',false);");
    assert_eq!(still.len(), 1, "{still:?}");
}

#[test]
fn corpus2_options_optimset_bare_named_overlay_shadowing_and_solver() {
    let mut e = Engine::default();
    // Reference: MATLAB R2024b optimset (8 base + 50 Optimization Toolbox = 58 fields).
    run(
        &mut e,
        "opt = optimset; opt_p = optimset();\
         n_bare = numfields(opt); n_paren = numfields(opt_p);\
         eq_fields = 1*isequal(fieldnames(opt), fieldnames(opt_p));\
         em_bare = 1*isempty(opt.GradObj);\
         named = optimset('Display', 'off');\
         disp_txt = named.Display; disp_ischar = 1*ischar(named.Display);\
         grad_unset = 1*isempty(named.GradObj); tolx_unset = 1*isempty(named.TolX);\
         ci = optimset('display', 'iter'); n_ci = numfields(ci); ci_disp = ci.Display;\
         o1 = optimset('Display', 'off', 'TolX', 1e-6, 'MaxIter', 50);\
         o2 = optimset('TolX', 1e-8, 'MaxIter', [], 'GradObj', 'on');\
         m = optimset(o1, o2);\
         m_disp = m.Display; m_tolx = m.TolX; m_mi = m.MaxIter; m_go = m.GradObj;\
         m2 = optimset(o1, o2, 'MaxIter', 10, 'TolX', []);\
         m2_mi = m2.MaxIter; m2_tolx_em = 1*isempty(m2.TolX);\
         base = struct('CustomEmpty', []); overlay = struct('MaxIter', 7);\
         preserved = optimset(base, overlay); custom_present = 1*isfield(preserved, 'CustomEmpty');\
         custom_empty = 1*isempty(preserved.CustomEmpty); overridden = preserved.MaxIter;\
         [x_unc, f_unc, flag_unc] = fminunc(@(x) (x-2)^2, 0, opt);\
         [x_fms, f_fms, flag_fms] = fminsearch(@(x) (x-2)^2, 0, opt);\
         [x_fz, f_fz, flag_fz] = fzero(@(x) x-2, [0 3], opt);\
         optimset = 99; shadowed = optimset;",
    );
    assert_eq!(scalar(&e, "n_bare"), 58.);
    assert_eq!(scalar(&e, "n_paren"), 58.);
    assert_eq!(scalar(&e, "eq_fields"), 1.);
    assert_eq!(scalar(&e, "em_bare"), 1.);
    assert_eq!(text(&e, "disp_txt"), "off");
    assert_eq!(scalar(&e, "disp_ischar"), 1.);
    assert_eq!(scalar(&e, "grad_unset"), 1.);
    assert_eq!(scalar(&e, "tolx_unset"), 1.);
    assert_eq!(scalar(&e, "n_ci"), 58.);
    assert_eq!(text(&e, "ci_disp"), "iter");
    assert_eq!(text(&e, "m_disp"), "off");
    assert_eq!(scalar(&e, "m_tolx"), 1e-8);
    assert_eq!(scalar(&e, "m_mi"), 50.);
    assert_eq!(text(&e, "m_go"), "on");
    assert_eq!(scalar(&e, "m2_mi"), 10.);
    assert_eq!(scalar(&e, "m2_tolx_em"), 1.);
    assert_eq!(scalar(&e, "custom_present"), 1.);
    assert_eq!(scalar(&e, "custom_empty"), 1.);
    assert_eq!(scalar(&e, "overridden"), 7.);
    assert_eq!(scalar(&e, "flag_unc"), 1.);
    assert!((scalar(&e, "x_unc") - 2.).abs() < 1e-5);
    assert_eq!(scalar(&e, "flag_fms"), 1.);
    assert!((scalar(&e, "x_fms") - 2.).abs() < 1e-3);
    assert_eq!(scalar(&e, "flag_fz"), 1.);
    assert!((scalar(&e, "x_fz") - 2.).abs() < 1e-10);
    assert_eq!(scalar(&e, "shadowed"), 99.);
}

#[test]
fn residual_options_jacobian_solves_and_algorithm_warnings() {
    let mut e = Engine::default();
    run(
        &mut e,
        "box_res = @(x)deal(x-[-1;2],eye(2)); rect_res = @(x)deal([x-2;2*(x-2)],[1;2]);",
    );

    let warnings = run(
        &mut e,
        "opts = optimoptions(@lsqnonlin,'Display','off','Algorithm','trust-region-reflective',\
        'Jacobian','on','MaxFunEvals',2); [msg,id] = lastwarn;",
    );
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(text(&e, "id"), "optimoptions:lsqnonlin:Algorithm");
    run(
        &mut e,
        "requested = opts.Algorithm; supplied = opts.SpecifyObjectiveGradient;",
    );
    assert_eq!(text(&e, "requested"), "trust-region-reflective");
    assert_eq!(text(&e, "supplied"), "on");
    assert!(
        text(&e, "msg").contains("trust-region-reflective")
            && text(&e, "msg").contains("levenberg-marquardt")
    );
    // With two evaluations the finite-difference solver cannot even form the
    // first full Jacobian. Projection of [-1;2] onto the bounds gives [0;2].
    let solve = run(
        &mut e,
        "[x,rn,r,flag,out] = lsqnonlin(box_res,[3;3],[0;0],[],opts); alg = out.algorithm; fc = out.funcCount;",
    );
    assert!(solve.is_empty(), "{solve:?}");
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[0., 2.], 1e-12, "bounded x");
    close(&values(&e, "r"), &[1., 0.], 1e-12, "bounded residual");
    close(&values(&e, "rn"), &[1.], 1e-12, "bounded resnorm");
    assert_eq!(scalar(&e, "fc"), 2.);
    assert_eq!(
        text(&e, "alg"),
        "levenberg-marquardt (projected onto the bounds)"
    );

    let build = run(
        &mut e,
        "opts = optimoptions(@fsolve,'Display','off','Algorithm','trust-region-dogleg','SpecifyObjectiveGradient',true);",
    );
    assert!(build.is_empty(), "{build:?}");
    let square = run(
        &mut e,
        "[x,r,flag,out,J] = fsolve(box_res,[0;0],opts); alg = out.algorithm;",
    );
    assert!(square.is_empty(), "{square:?}");
    assert_eq!(text(&e, "alg"), "trust-region-dogleg");
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[-1., 2.], 1e-12, "square root");

    let rectangular =
        "[x,r,flag,out,J] = fsolve(rect_res,0,opts); alg = out.algorithm; [msg,id] = lastwarn;";
    let warning = run(&mut e, rectangular);
    assert_eq!(warning.len(), 1, "{warning:?}");
    assert_eq!(text(&e, "id"), "fsolve:NonSquareSystem");
    assert!(
        text(&e, "msg").contains("trust-region-dogleg")
            && text(&e, "msg").contains("levenberg-marquardt")
    );
    assert_eq!(text(&e, "alg"), "levenberg-marquardt");
    assert_eq!(scalar(&e, "flag"), 1.);
    close(&values(&e, "x"), &[2.], 1e-12, "rectangular root");
    close(&values(&e, "J"), &[1., 2.], 0., "residual Jacobian");
    run(
        &mut e,
        "warning('off','fsolve:NonSquareSystem'); lastwarn('','');",
    );
    assert!(run(&mut e, rectangular).is_empty());
    assert_eq!(text(&e, "id"), "fsolve:NonSquareSystem");
    let default_dogleg = run(
        &mut e,
        "warning('on','fsolve:NonSquareSystem'); opts = optimoptions(opts,'Algorithm',[]);",
    );
    assert!(default_dogleg.is_empty(), "{default_dogleg:?}");
    assert_eq!(run(&mut e, rectangular).len(), 1);
    assert_eq!(text(&e, "id"), "fsolve:NonSquareSystem");

    let trust = run(
        &mut e,
        "opts = optimoptions('fsolve','Algorithm','trust-region','Jacobian','on','Display','off'); [msg,id] = lastwarn;",
    );
    assert_eq!(trust.len(), 1, "{trust:?}");
    assert_eq!(text(&e, "id"), "optimoptions:fsolve:Algorithm");
    assert!(text(&e, "msg").contains("levenberg-marquardt"));
    assert!(run(&mut e, rectangular).is_empty());
    assert_eq!(text(&e, "alg"), "levenberg-marquardt");

    let err = fails(&mut e, "fsolve(@(x)deal([x-2;2*(x-2)],[1,2]),0,opts);");
    assert!(err.contains("2-by-1") && err.contains("1-by-2"), "{err}");
}

#[test]
fn residual_options_r2024b_names_values_and_aliases() {
    let mut e = Engine::default();
    for (solver, call) in [
        ("fsolve", "[x,r,flag,out] = fsolve(@(x)x-2,0,o);"),
        (
            "lsqnonlin",
            "[x,rn,r,flag,out] = lsqnonlin(@(x)x-2,0,[],[],o);",
        ),
    ] {
        for (key, given, value) in [
            ("CheckGradients", "DerivativeCheck", "'on'"),
            ("Diagnostics", "Diagnostics", "'off'"),
            ("DiffMinChange", "DiffMinChange", "0"),
            ("DiffMaxChange", "DiffMaxChange", "Inf"),
            ("FiniteDifferenceStepSize", "FinDiffRelStep", "1e-4"),
            ("FunValCheck", "FunValCheck", "'on'"),
            ("JacobianMultiplyFcn", "jAcObMuLt", "@(j,y,flag) y"),
            ("JacobPattern", "JacobPattern", "sparse(1)"),
            ("MaxPCGIter", "MaxPCGIter", "3"),
            ("PrecondBandWidth", "PrecondBandWidth", "Inf"),
            (
                "SubproblemAlgorithm",
                "SubproblemAlgorithm",
                "'factorization'",
            ),
            ("TolPCG", "TolPCG", "0.1"),
            ("InitDamping", "InitDamping", "0.01"),
            ("ScaleProblem", "ScaleProblem", "'jacobian'"),
            ("TypicalX", "TypicalX", "1"),
            ("UseParallel", "UseParallel", "true"),
        ] {
            let warning = run(
                &mut e,
                &format!(
                    "o = optimoptions('{solver}','Display','off','{given}',{value},'{key}',{value}); [msg,id] = lastwarn;"
                ),
            );
            assert_eq!(warning.len(), 1, "{solver} {given}: {warning:?}");
            assert!(text(&e, "msg").contains(key), "{warning:?}");
            assert_eq!(text(&e, "id"), format!("optimoptions:{solver}:{key}"));
            let solve = run(&mut e, call);
            assert!(solve.is_empty(), "{given}: {solve:?}");
            assert_eq!(scalar(&e, "flag"), 1.);
            close(&values(&e, "x"), &[2.], 1e-8, given);
        }
        let empty = run(
            &mut e,
            &format!(
                "o = optimoptions('{solver}','Algorithm',[],'JacobMult',[],'FinDiffRelStep',[],'Display','off','MaxIter',0); {call} it = out.iterations;"
            ),
        );
        assert!(empty.is_empty(), "{empty:?}");
        assert_eq!(scalar(&e, "it"), 0.);
        assert_eq!(scalar(&e, "x"), 0.);
        for (name, bad) in [
            ("Algorithm", "'sqp'"),
            ("SubproblemAlgorithm", "'dogleg'"),
            ("Diagnostics", "true"),
            ("Jacobian", "NaN"),
            ("SpecifyObjectiveGradient", "2"),
            ("CheckGradients", "-1"),
            ("UseParallel", "Inf"),
            ("FiniteDifferenceStepSize", "[1 2;3 4]"),
            ("DiffMinChange", "-1"),
            ("DiffMaxChange", "0"),
            ("FunValCheck", "'yes'"),
            ("JacobianMultiplyFcn", "1"),
            ("JacobPattern", "[1 NaN]"),
            ("MaxPCGIter", "0"),
            ("PrecondBandWidth", "-1"),
            ("TolPCG", "Inf"),
            ("InitDamping", "0"),
            ("ScaleProblem", "true"),
            ("TypicalX", "NaN"),
        ] {
            run(&mut e, "lastwarn('sentinel','sentinel:id');");
            let error = fails(
                &mut e,
                &format!("optimoptions('{solver}','CheckGradients',true,'{name}',{bad});"),
            );
            assert!(
                error.contains(if name == "Jacobian" {
                    "SpecifyObjectiveGradient"
                } else {
                    name
                }),
                "{error}"
            );
            run(&mut e, "[msg,id] = lastwarn;");
            assert_eq!(text(&e, "id"), "sentinel:id");
        }
    }
    for (solver, option) in [
        ("fsolve", "'Algorithm','trust-region-reflective'"),
        ("fsolve", "'Algorithm','interior-point'"),
        ("lsqnonlin", "'Algorithm','trust-region'"),
        ("lsqnonlin", "'Algorithm','trust-region-dogleg'"),
        ("fsolve", "'InitBarrierParam',0.1"),
        ("fsolve", "'BarrierParamUpdate','monotone'"),
        ("lsqnonlin", "'BarrierParamUpdate','cg'"),
        ("lsqnonlin", "'InitBarrierParam',-1"),
        ("fsolve", "'GradObj','on'"),
        ("lsqnonlin", "'GradObj','on'"),
        ("lsqnonlin", "'Jacobien','on'"),
        ("fsolve", "'ScaleProbem','jacobian'"),
        ("fminunc", "'JacobMult',@cos"),
        ("lsqcurvefit", "'InitDamping',1"),
        ("fmincon", "'InitDamping',1"),
        ("particleswarm", "'JacobPattern',1"),
    ] {
        let error = fails(&mut e, &format!("optimoptions('{solver}',{option});"));
        assert!(
            error.contains(option.split('\'').nth(1).unwrap()),
            "{solver} {error}"
        );
    }
    let ip = run(
        &mut e,
        "o = optimoptions('lsqnonlin','Algorithm','interior-point','InitBarrierParam',0.1,'BarrierParamUpdate','monotone','Display','off');",
    );
    assert_eq!(ip.len(), 3, "{ip:?}");
    assert!(
        run(
            &mut e,
            "[x,rn,r,flag,out] = lsqnonlin(@(x)x-2,0,[],[],o); alg = out.algorithm;"
        )
        .is_empty()
    );
    assert_eq!(text(&e, "alg"), "levenberg-marquardt");
    close(&values(&e, "x"), &[2.], 1e-12, "interior-point fallback");
}

#[test]
fn residual_options_keep_named_dispatch_with_empty_constrained_defaults() {
    let mut e = Engine::default();
    for (solver, solve) in [
        ("lsqnonlin", "[x,~,~,flag] = lsqnonlin(@(x)x-2,0,[],[],o);"),
        ("fsolve", "[x,~,flag] = fsolve(@(x)x-2,0,o);"),
    ] {
        let damping = "'InitDamping',0.1";
        let default = "'ConstraintTolerance',[]";
        let baseline = run(&mut e, &format!("optimoptions('{solver}',{damping});"));
        assert_eq!(baseline.len(), 1, "{baseline:?}");
        let warning_id = format!("optimoptions:{solver}:InitDamping");

        for named in [format!("'{solver}'"), format!("@{solver}")] {
            for pairs in [
                format!("{default},{damping}"),
                format!("{damping},{default}"),
            ] {
                let warning = run(
                    &mut e,
                    &format!(
                        "o = optimoptions({named},{pairs}); [msg,id] = lastwarn; \
                         tol = o.ConstraintTolerance; damping = o.InitDamping;"
                    ),
                );
                assert_eq!(warning, baseline, "{named} {pairs}");
                assert_eq!(text(&e, "id"), warning_id);
                assert!(values(&e, "tol").is_empty());
                assert_eq!(scalar(&e, "damping"), 0.1);
            }
        }

        let suppressed = run(
            &mut e,
            &format!(
                "warning('off','{warning_id}'); lastwarn('',''); \
                 o = optimoptions('{solver}',{default},{damping}); [msg,id] = lastwarn;"
            ),
        );
        assert!(suppressed.is_empty(), "{suppressed:?}");
        assert_eq!(text(&e, "id"), warning_id);
        assert!(text(&e, "msg").contains("InitDamping"));
        for update in [
            "'InitDamping',0.2",
            "'ConstraintTolerance',[],'InitDamping',0.2",
        ] {
            assert!(
                run(
                    &mut e,
                    &format!("u=optimoptions(o,{update}); d=u.InitDamping;")
                )
                .is_empty()
            );
            assert_eq!(scalar(&e, "d"), 0.2);
        }
        let output = run(
            &mut e,
            &format!("warning('on','{warning_id}'); o = optimoptions(o,'Display','off'); {solve}"),
        );
        assert!(output.is_empty(), "{output:?}");
        assert_eq!(scalar(&e, "flag"), 1.);
        close(&values(&e, "x"), &[2.], 1e-8, solver);

        for (pairs, expected) in [
            (
                "'InitDamping',0.1,'ConstraintTolerance',1e-6",
                "not implemented",
            ),
            ("'ConstraintTolerance',[],'InitDamping'", "name-value pairs"),
            (
                "'ConstraintTolerance',[],'InitDamping',0",
                "invalid numeric value for InitDamping",
            ),
            (
                "'ConstraintTolerance',[],'InitDamping',0.1,42,[]",
                "option names",
            ),
            (
                "'ConstraintToleranc',[],'InitDamping',0.1",
                "unknown option 'ConstraintToleranc'",
            ),
        ] {
            run(&mut e, "lastwarn('sentinel','sentinel:id');");
            let error = fails(&mut e, &format!("optimoptions('{solver}',{pairs});"));
            assert!(error.contains(expected), "{error}");
            run(&mut e, "[msg,id] = lastwarn;");
            assert_eq!(text(&e, "id"), "sentinel:id");
        }
    }

    let error = fails(&mut e, "optimoptions(struct(),'InitDamping',0.1);");
    assert!(error.contains("does not name a solver"), "{error}");
    let error = fails(
        &mut e,
        "optimoptions(struct('ConstraintTolerance',[]),'InitDamping',0.1);",
    );
    assert!(
        error.contains("'InitDamping' is solver-specific")
            && error.contains("does not name a solver"),
        "{error}"
    );
}

#[test]
fn residual_options_suppression_and_untagged_updates() {
    let mut e = Engine::default();
    let suppressed = run(
        &mut e,
        "warning('off','optimoptions:lsqnonlin:CheckGradients'); lastwarn('',''); \
        o = optimoptions('lsqnonlin','Display','off','DerivativeCheck','on','CheckGradients',true); [msg,id] = lastwarn;",
    );
    assert!(suppressed.is_empty(), "{suppressed:?}");
    assert_eq!(text(&e, "id"), "optimoptions:lsqnonlin:CheckGradients");
    assert!(text(&e, "msg").contains("CheckGradients"));
    let other = run(&mut e, "f = optimoptions('fsolve','DerivativeCheck','on');");
    assert_eq!(other.len(), 1, "{other:?}");
    let copied = run(
        &mut e,
        "p = optimoptions(o,'MaxIter',20,'DerivativeCheck','off'); [x,rn] = lsqnonlin(@(x)x-2,1,[],[],p);",
    );
    assert!(copied.is_empty(), "{copied:?}");
    close(&values(&e, "x"), &[2.], 1e-12, "copy");
    assert!(fails(&mut e, "optimoptions(p,'DerivativeCheck','loud');").contains("CheckGradients"));
    assert!(fails(&mut e, "optimoptions(p,'JacobMult',@sin);").contains("does not name a solver"));
    let ambiguous = run(
        &mut e,
        "p = optimoptions(p,'Algorithm','trust-region-reflective'); [msg,id] = lastwarn;",
    );
    assert_eq!(ambiguous.len(), 1, "{ambiguous:?}");
    assert_eq!(text(&e, "id"), "optimoptions:Algorithm");
    assert!(text(&e, "msg").contains("without a solver name"));
    assert!(run(&mut e,"p = optimoptions(p,'Algorithm','trust-region-reflective'); [x,rn,r,flag,out] = lsqnonlin(@(x)x-2,1,[],[],p); alg = out.algorithm;").is_empty());
    assert_eq!(text(&e, "alg"), "levenberg-marquardt");
    let reset = run(
        &mut e,
        "f = optimoptions('fsolve','ScaleProblem','jacobian','ScaleProblem',[],'Algorithm','trust-region','Algorithm','trust-region-dogleg');",
    );
    assert!(reset.is_empty(), "{reset:?}");
}

#[test]
fn residual_options_positive_tolfun_update_sets_both_tolerances() {
    // Unlike the QP empty reset (optim_constrained), a positive TolFun on a
    // returned fsolve/lsqnonlin struct sets FunctionTolerance and
    // OptimalityTolerance. atan(x)=1 has the root tan(1); from 0 the defaults
    // take more iterations than .01, both with flag 1.
    let mut e = Engine::default();
    for (solver, solve) in [
        ("fsolve", "[x,~,flag,out] = fsolve(@(x)atan(x)-1,0,u);"),
        (
            "lsqnonlin",
            "[x,~,~,flag,out] = lsqnonlin(@(x)atan(x)-1,0,[],[],u);",
        ),
    ] {
        run(
            &mut e,
            &format!(
                "o = optimoptions('{solver}','Display','off'); p = optimoptions(o,'TolFun',.01); \
                 tols = [p.FunctionTolerance p.OptimalityTolerance];"
            ),
        );
        assert_eq!(values(&e, "tols"), [0.01, 0.01], "{solver}");
        let mut iterations = Vec::new();
        for (options, tol) in [("o", 1e-8), ("p", 1e-2), ("o", 1e-8)] {
            assert!(
                run(
                    &mut e,
                    &format!("u = {options}; {solve} it = out.iterations;")
                )
                .is_empty()
            );
            assert_eq!(scalar(&e, "flag"), 1., "{solver} {options}");
            close(&values(&e, "x"), &[1f64.tan()], tol, solver);
            iterations.push(scalar(&e, "it"));
        }
        assert!(
            iterations[1] < iterations[0] && iterations[2] == iterations[0],
            "{solver} default/.01/default iterations: {iterations:?}"
        );
    }
}
