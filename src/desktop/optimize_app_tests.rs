// Optimize app (apps/optimize_app.rs): every solver's generated script
// reproduces what the app shows, solver lists follow the problem, skipped
// argument slots, non-default options only, Stop, export without
// clobbering, errors and a rendered tab.
mod optimize_app_tests {
    use super::*;
    use apps::optimize_app::{
        Constraint, Constraints, DisplayLevel, Opt, Optimize, Problem, Slot, Solution, Solver,
        Source, View, solvers,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    /// Run, then one private run synchronously, delivered to the app.
    fn solve(host: &mut Host<Optimize>, ws: &Snapshot) -> Result<(), String> {
        host.app.run();
        host.app.prepare(ws);
        let request = host.request(ws)?;
        let outputs = apps::run(&mut matrust::Engine::default(), request)?;
        host.app.receive(&outputs)
    }
    fn shown(host: &Host<Optimize>) -> &Solution {
        host.app.result.as_ref().expect("a result")
    }
    fn numbers(engine: &matrust::Engine, name: &str) -> Vec<f64> {
        match engine.array(name) {
            Some(matrust::Array::Numeric(m)) => m.data().to_vec(),
            other => panic!("{name}: {other:?}"),
        }
    }
    /// Runs `script` in a fresh Engine (after `setup`): x and fval.
    fn reproduce(setup: &str, script: &str) -> (Vec<f64>, Vec<f64>) {
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        engine
            .execute(script)
            .unwrap_or_else(|e| panic!("{script}\n{e}"));
        (
            numbers(&engine, "solution"),
            numbers(&engine, "objectiveValue"),
        )
    }
    fn workspace(setup: &str) -> Snapshot {
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    /// Solves, checks the script against a fresh Engine and returns it.
    fn solved_script(host: &mut Host<Optimize>, ws: &Snapshot, setup: &str) -> String {
        solve(host, ws).unwrap_or_else(|e| panic!("{e}\n{:?}", host.app.script(ws)));
        let script = host.app.script(ws).unwrap();
        let (x, fval) = reproduce(setup, &script);
        let r = shown(host);
        assert_eq!(x, r.x, "{script}");
        assert_eq!(fval, r.fval, "{script}");
        script
    }
    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn optimize_app_rosenbrock_with_fminunc_and_fminsearch() {
        let ws = Snapshot::default();
        let mut host = Host::new(Optimize::default());
        let script = solved_script(&mut host, &ws, "");
        assert_eq!(
            script,
            "% Minimize a function with fminunc\n\
             fun = @(x) (x(1)-1)^2 + 100*(x(2)-x(1)^2)^2;\n\
             x0 = [-1 2];\n\
             [solution, objectiveValue, exitflag, output] = fminunc(fun, x0);\n\
             solution\nobjectiveValue\n"
        );
        let r = shown(&host);
        assert_eq!(r.solver, Solver::Fminunc);
        assert!(
            close(r.x[0], 1., 1e-3) && close(r.x[1], 1., 1e-3),
            "{:?}",
            r.x
        );
        assert_eq!(r.exitflag, 1.);
        assert!(r.iterations.is_some_and(|k| k > 10.));
        // The recorded second solve saw every evaluation the solver counted.
        let best = &r.progress.best;
        assert_eq!(best.last().unwrap()[0], r.evaluations.unwrap());
        assert!(best.windows(2).all(|w| w[1][1] <= w[0][1]));
        assert_eq!(best[0][1], 104., "f(x0) = 4 + 100");
        // The path starts at x0 and ends at the solution (or a finite
        // difference probe next to it, when that one is lower), on a 2-D map.
        assert_eq!(r.progress.path.first().unwrap(), &vec![-1., 2.]);
        let last = r.progress.path.last().unwrap();
        assert!(close(last[0], r.x[0], 1e-6) && close(last[1], r.x[1], 1e-6));
        let map = r.map.as_ref().expect("a map for two variables");
        assert!(map.two_d() && map.values.len() == map.x.len() * map.y.len());
        assert!(map.x[0] <= -1. && *map.x.last().unwrap() >= 1.);
        assert!(map.values.iter().all(|v| v.is_finite()));
        let rows = host.app.results();
        assert_eq!(rows[0], ("Solver", "fminunc".to_owned()));
        assert_eq!(rows[3], ("Exit flag", "1".to_owned()));
        assert!(
            host.app
                .summary()
                .unwrap()
                .starts_with("fminunc · Objective")
        );
        // fminsearch: three outputs, no options, evaluations from the record.
        host.app.spec.solver = Some(Solver::Fminsearch);
        let script = solved_script(&mut host, &ws, "");
        assert!(
            script.contains(
                "[solution, objectiveValue, exitflag] = fminsearch(fun, x0);\nsolution\n"
            ),
            "{script}"
        );
        let r = shown(&host);
        assert_eq!(r.solver, Solver::Fminsearch);
        assert!(close(r.x[0], 1., 1e-3) && close(r.x[1], 1., 1e-3));
        assert_eq!(r.iterations, None);
        assert_eq!(
            r.evaluations,
            Some(r.progress.best.last().unwrap()[0]),
            "counted by the recorder"
        );
        assert_eq!(
            host.app.exported(&ws),
            ["solution", "objectiveValue", "exitflag", "fun", "x0"]
        );
    }

    #[test]
    fn optimize_app_bounded_fmincon_with_a_nonlinear_constraint() {
        let ws = Snapshot::default();
        let mut host = Host::new(Optimize::default());
        let spec = &mut host.app.spec;
        spec.set(Slot::X0, "[0 0]");
        spec.constraints.bounds = true;
        spec.constraints.nonlinear = true;
        spec.set(Slot::Lb, "[-2 -2]");
        spec.set(Slot::Ub, "[2 2]");
        spec.nonlcon.expression = "x(1)^2 + x(2)^2 - 1.5".into();
        assert_eq!(spec.solver(&ws), Solver::Fmincon, "recommended");
        let script = solved_script(&mut host, &ws, "");
        assert!(
            script.contains(
                "x0 = [0 0];\nlb = [-2 -2];\nub = [2 2];\n\
                 nonlcon = @(x) deal(x(1)^2 + x(2)^2 - 1.5, []);\n\
                 [solution, objectiveValue, exitflag, output] = \
                 fmincon(fun, x0, [], [], [], [], lb, ub, nonlcon);\n"
            ),
            "{script}"
        );
        let r = shown(&host);
        // The optimum lies on the circle x1^2 + x2^2 = 1.5.
        assert!(
            close(r.x[0].powi(2) + r.x[1].powi(2), 1.5, 1e-5),
            "{:?}",
            r.x
        );
        assert!(r.exitflag > 0.);
        assert!(r.violation.is_some_and(|v| v < 1e-6));
        let (_, mu) = r
            .lambda
            .iter()
            .find(|(name, _)| *name == "ineqnonlin")
            .expect("multiplier of the active constraint");
        assert!(mu[0] > 0.);
        assert!(r.lambda.iter().any(|(name, _)| *name == "lower"));
        // Progress tracks the violation; the map greys the infeasible corners.
        assert!(!r.progress.violation.is_empty());
        assert!(r.progress.violation.last().unwrap()[1] <= 1e-6);
        let map = r.map.as_ref().unwrap();
        assert!(map.infeasible.iter().any(|b| *b) && map.infeasible.iter().any(|b| !*b));
        assert!(
            host.app
                .results()
                .iter()
                .any(|(label, _)| *label == "Constraint violation")
        );
    }

    #[test]
    fn optimize_app_linprog_intlinprog_and_quadprog() {
        let ws = Snapshot::default();
        let mut host = Host::new(Optimize::default());
        let spec = &mut host.app.spec;
        spec.problem = Problem::Linear;
        spec.set(Slot::F, "[-1 -2]");
        spec.constraints.inequalities = true;
        spec.constraints.bounds = true;
        spec.set(Slot::A, "[1 1; 1 -1]");
        spec.set(Slot::B, "[4; 2]");
        spec.set(Slot::Lb, "[0 0]");
        let script = solved_script(&mut host, &ws, "");
        assert_eq!(
            script,
            "% Linear program with linprog\n\
             f = [-1 -2];\nlb = [0 0];\nA = [1 1; 1 -1];\nb = [4; 2];\n\
             [solution, objectiveValue, exitflag, output] = linprog(f, A, b, [], [], lb);\n\
             solution\nobjectiveValue\n"
        );
        let r = shown(&host);
        assert!(
            close(r.x[0], 0., 1e-6) && close(r.x[1], 4., 1e-6),
            "{:?}",
            r.x
        );
        assert!(close(r.fval[0], -8., 1e-6));
        assert!(r.lambda.iter().any(|(name, _)| *name == "ineqlin"));
        assert!(
            r.progress.best.is_empty(),
            "a linear program records no evaluations"
        );
        assert!(r.map.as_ref().is_some_and(|m| m.two_d()));
        // intlinprog: x1 + x2 <= 4.5 rounds the optimum down to integers.
        host.app.spec.problem = Problem::MixedInteger;
        host.app.spec.set(Slot::Intcon, "[1 2]");
        host.app.spec.set(Slot::B, "[4.5; 2]");
        let script = solved_script(&mut host, &ws, "");
        assert!(
            script.contains("intlinprog(f, intcon, A, b, [], [], lb);"),
            "{script}"
        );
        let r = shown(&host);
        assert_eq!(r.solver, Solver::Intlinprog);
        assert!(
            close(r.x[0], 0., 1e-9) && close(r.x[1], 4., 1e-9),
            "{:?}",
            r.x
        );
        assert!(close(r.fval[0], -8., 1e-9));
        // quadprog: min x1^2 + x2^2 - 2 x1 - 5 x2 with x1 + 2 x2 <= 3; the
        // KKT point is (0.4, 1.3).
        let spec = &mut host.app.spec;
        spec.problem = Problem::Quadratic;
        spec.constraints.bounds = false;
        spec.set(Slot::H, "[2 0; 0 2]");
        spec.set(Slot::F, "[-2 -5]");
        spec.set(Slot::A, "[1 2]");
        spec.set(Slot::B, "3");
        let script = solved_script(&mut host, &ws, "");
        assert!(script.contains("quadprog(H, f, A, b);"), "{script}");
        let r = shown(&host);
        assert!(
            close(r.x[0], 0.4, 1e-6) && close(r.x[1], 1.3, 1e-6),
            "{:?}",
            r.x
        );
        assert!(close(r.fval[0], 0.4 * 0.4 + 1.3 * 1.3 - 0.8 - 6.5, 1e-6));
    }

    #[test]
    fn optimize_app_fsolve_fzero_and_lsqcurvefit() {
        let mut host = Host::new(Optimize::default());
        let ws = Snapshot::default();
        // A 2x2 system: the circle of radius 2 meets the line x1 = x2.
        let spec = &mut host.app.spec;
        spec.problem = Problem::Equations;
        spec.system = true;
        spec.objective.expression = "[x(1)^2 + x(2)^2 - 4; x(1) - x(2)]".into();
        spec.set(Slot::X0, "[1 1]");
        let script = solved_script(&mut host, &ws, "");
        assert!(
            script.contains("[solution, objectiveValue, exitflag, output] = fsolve(fun, x0);"),
            "{script}"
        );
        let r = shown(&host);
        let root = 2_f64.sqrt();
        assert!(
            close(r.x[0], root, 1e-9) && close(r.x[1], root, 1e-9),
            "{:?}",
            r.x
        );
        assert_eq!(r.fval.len(), 2);
        assert_eq!(r.value_row().0, "‖F(x)‖");
        // One unknown: fzero is recommended; Brent's root of x^3 - 2x - 5.
        host.app.spec.system = false;
        host.app.spec.objective.expression = "x^3 - 2*x - 5".into();
        host.app.spec.set(Slot::X0, "2");
        assert_eq!(host.app.spec.solver(&ws), Solver::Fzero);
        let script = solved_script(&mut host, &ws, "");
        assert!(
            script.contains("x0 = 2;\n[solution, objectiveValue, exitflag] = fzero(fun, x0);"),
            "{script}"
        );
        let r = shown(&host);
        assert!(close(r.x[0], 2.094_551_481_542_326_5, 1e-12), "{:?}", r.x);
        let map = r.map.as_ref().expect("a one-variable map");
        assert!(!map.two_d() && map.x.len() == apps::optimize_app::LINE_POINTS);
        // lsqcurvefit: ydata from the workspace, xdata typed as an expression.
        let ws = workspace("yd = 2*exp(-0.7*(0:0.5:3));");
        let spec = &mut host.app.spec;
        spec.problem = Problem::CurveFit;
        spec.objective.expression = "x(1)*exp(-x(2)*xdata)".into();
        spec.set(Slot::X0, "[1 1]");
        spec.set(Slot::Xdata, "0:0.5:3");
        spec.set(Slot::Ydata, "yd");
        let script = solved_script(&mut host, &ws, "yd = 2*exp(-0.7*(0:0.5:3));");
        assert!(
            script.contains(
                "fun = @(x, xdata) x(1)*exp(-x(2)*xdata);\nx0 = [1 1];\nxdata = 0:0.5:3;\n\
                 [solution, objectiveValue, ~, exitflag, output] = lsqcurvefit(fun, x0, xdata, yd);"
            ),
            "{script}"
        );
        assert_eq!(host.app.inputs(), ["yd"]);
        let r = shown(&host);
        assert!(
            close(r.x[0], 2., 1e-6) && close(r.x[1], 0.7, 1e-6),
            "{:?}",
            r.x
        );
        let fit = r.fit.as_ref().expect("data and model");
        assert_eq!(fit.data.len(), 7);
        assert_eq!(fit.curve.len(), apps::optimize_app::LINE_POINTS);
        assert!(close(fit.curve[0][1], 2., 1e-5));
    }

    #[test]
    fn optimize_app_ga_with_a_fixed_seed_reproduces() {
        let ws = Snapshot::default();
        let mut host = Host::new(Optimize::default());
        let spec = &mut host.app.spec;
        spec.problem = Problem::Global;
        spec.constraints.bounds = true;
        spec.set(Slot::Lb, "[-5 -5]");
        spec.set(Slot::Ub, "[5 5]");
        spec.solver = Some(Solver::Ga);
        spec.seed = "7".into();
        let script = solved_script(&mut host, &ws, "");
        assert!(
            script.contains(
                "lb = [-5 -5];\nub = [5 5];\nrng(7);\n\
                 [solution, objectiveValue, exitflag, output] = ga(fun, 2, [], [], [], [], lb, ub);"
            ),
            "{script}"
        );
        assert!(
            !script.contains("x0"),
            "ga reads only the number of variables"
        );
        let first = shown(&host).clone();
        assert!(first.fval[0] < 0.1, "{:?}", first.fval);
        // The recorded solve repeats the seeded run.
        assert_eq!(
            first.progress.best.last().unwrap()[0],
            first.evaluations.unwrap()
        );
        solve(&mut host, &ws).unwrap();
        assert_eq!(shown(&host).x, first.x);
        // Another seed is another run.
        host.app.spec.seed = "8".into();
        solve(&mut host, &ws).unwrap();
        assert_ne!(shown(&host).x, first.x);
        assert!(host.app.script(&ws).unwrap().contains("rng(8);"));
        // The number of variables can be typed.
        host.app.spec.nvars = "3".into();
        host.app.spec.set(Slot::Lb, "[-5 -5 -5]");
        host.app.spec.set(Slot::Ub, "[5 5 5]");
        host.app.spec.objective.expression = "sum((x - [1 2 3]).^2)".into();
        let script = solved_script(&mut host, &ws, "");
        assert!(script.contains("ga(fun, 3, "), "{script}");
    }

    #[test]
    fn optimize_app_solver_lists_follow_problem_and_constraints() {
        use Solver::*;
        let none = Constraints::default();
        let with = |kinds: &[Constraint]| {
            let mut c = Constraints::default();
            for kind in kinds {
                c.set(*kind, true);
            }
            c
        };
        let bounds = with(&[Constraint::Bounds]);
        let nonlinear = with(&[Constraint::Nonlinear]);
        let linear = with(&[Constraint::Inequalities, Constraint::Bounds]);
        let equalities = with(&[Constraint::Equalities]);
        let m = Problem::Minimize;
        assert_eq!(
            solvers(m, none, false, false),
            [Fminunc, Fminsearch, Fmincon]
        );
        assert_eq!(solvers(m, bounds, false, true), [Fminbnd, Fmincon]);
        assert_eq!(solvers(m, bounds, false, false), [Fmincon]);
        assert_eq!(solvers(m, nonlinear, false, false), [Fmincon]);
        assert_eq!(solvers(m, linear, false, true), [Fmincon]);
        assert_eq!(solvers(m, equalities, false, false), [Fmincon]);
        let e = Problem::Equations;
        assert_eq!(solvers(e, none, false, true), [Fzero, Fsolve]);
        assert_eq!(solvers(e, none, true, false), [Fsolve]);
        assert_eq!(
            solvers(e, nonlinear, true, false),
            [Fsolve],
            "no constraints"
        );
        let g = Problem::Global;
        assert_eq!(solvers(g, nonlinear, false, false), [Ga, Patternsearch]);
        assert_eq!(solvers(g, linear, false, false), [Ga, Patternsearch]);
        assert_eq!(
            solvers(g, bounds, false, false),
            [Particleswarm, Ga, Patternsearch, Simulannealbnd]
        );
        assert_eq!(solvers(g, none, false, false)[0], Patternsearch);
        assert_eq!(solvers(Problem::Linear, nonlinear, false, false), [Linprog]);
        assert_eq!(
            solvers(Problem::MixedInteger, linear, false, false),
            [Intlinprog]
        );
        assert_eq!(solvers(Problem::Quadratic, none, false, false), [Quadprog]);
        assert_eq!(
            solvers(Problem::LeastSquares, linear, false, false),
            [Lsqnonlin]
        );
        assert_eq!(
            solvers(Problem::CurveFit, bounds, false, false),
            [Lsqcurvefit]
        );
        // Constraints a problem cannot have are ignored, not lost.
        assert_eq!(linear.within(Problem::LeastSquares), bounds);
        assert!(!Problem::Equations.allows(Constraint::Bounds));
        // A chosen solver that the constraints rule out falls back to the
        // recommended one; it comes back when they allow it again.
        let ws = Snapshot::default();
        let mut app = Optimize::default();
        app.spec.solver = Some(Fminsearch);
        assert_eq!(app.spec.solver(&ws), Fminsearch);
        app.spec.constraints.nonlinear = true;
        assert_eq!(app.spec.solvers(&ws), [Fmincon]);
        assert_eq!(app.spec.solver(&ws), Fmincon);
        app.spec.constraints.nonlinear = false;
        assert_eq!(app.spec.solver(&ws), Fminsearch);
        // fminbnd needs one variable: the start point decides.
        app.spec.constraints.bounds = true;
        assert_eq!(app.spec.solvers(&ws), [Fmincon]);
        app.spec.set(Slot::X0, "0.5");
        assert_eq!(app.spec.solvers(&ws), [Fminbnd, Fmincon]);
        let ws = workspace("s = 3; v = [1 2];");
        app.spec.set(Slot::X0, "s");
        assert_eq!(app.spec.solvers(&ws)[0], Fminbnd);
        app.spec.set(Slot::X0, "v");
        assert_eq!(app.spec.solvers(&ws), [Fmincon]);
    }

    #[test]
    fn optimize_app_argument_order_with_skipped_slots() {
        let ws = Snapshot::default();
        let call = |app: &mut Optimize| -> String {
            app.run();
            let script = app.script(&ws).unwrap();
            script
                .lines()
                .find(|l| l.contains(" = ") && l.contains('[') && l.contains("] = "))
                .unwrap()
                .split(" = ")
                .nth(1)
                .unwrap()
                .to_owned()
        };
        let mut app = Optimize::default();
        app.spec.constraints.bounds = true;
        app.spec.set(Slot::Lb, "[-2 -2]");
        app.spec.set(Slot::Ub, "[2 2]");
        assert_eq!(call(&mut app), "fmincon(fun, x0, [], [], [], [], lb, ub);");
        app.spec.constraints.bounds = false;
        app.spec.constraints.nonlinear = true;
        app.spec.nonlcon.expression = "x(1) + x(2) - 1".into();
        assert_eq!(
            call(&mut app),
            "fmincon(fun, x0, [], [], [], [], [], [], nonlcon);"
        );
        app.spec.constraints.nonlinear = false;
        app.spec.constraints.equalities = true;
        app.spec.set(Slot::Aeq, "[1 1]");
        app.spec.set(Slot::Beq, "1");
        app.spec.options.set(Opt::MaxIterations, "50");
        assert_eq!(
            call(&mut app),
            "fmincon(fun, x0, [], [], Aeq, beq, [], [], [], options);"
        );
        // Only an upper bound: lb is skipped.
        let mut app = Optimize::default();
        app.spec.problem = Problem::Linear;
        app.spec.set(Slot::F, "[1 1]");
        app.spec.constraints.bounds = true;
        app.spec.set(Slot::Ub, "[3 3]");
        assert_eq!(call(&mut app), "linprog(f, [], [], [], [], [], ub);");
        // quadprog's x0 slot stays empty before the options.
        app.spec.problem = Problem::Quadratic;
        app.spec.constraints.bounds = false;
        app.spec.set(Slot::H, "eye(2)");
        app.spec.options.display = Some(DisplayLevel::Off);
        assert_eq!(
            call(&mut app),
            "quadprog(H, f, [], [], [], [], [], [], [], options);"
        );
        let mut app = Optimize::default();
        app.spec.problem = Problem::CurveFit;
        app.spec.objective.expression = "x(1)*xdata".into();
        app.spec.set(Slot::Xdata, "[1 2 3]");
        app.spec.set(Slot::Ydata, "[2 4 6]");
        app.spec.options.set(Opt::FunctionTolerance, "1e-9");
        assert_eq!(
            call(&mut app),
            "lsqcurvefit(fun, x0, xdata, ydata, [], [], options);"
        );
        let mut app = Optimize::default();
        app.spec.problem = Problem::Global;
        app.spec.solver = Some(Solver::Ga);
        app.spec.constraints.bounds = true;
        app.spec.set(Slot::Ub, "[1 1]");
        assert_eq!(call(&mut app), "ga(fun, 2, [], [], [], [], [], ub);");
        app.spec.solver = Some(Solver::Simulannealbnd);
        assert_eq!(call(&mut app), "simulannealbnd(fun, x0, [], ub);");
        app.spec.solver = Some(Solver::Particleswarm);
        assert_eq!(call(&mut app), "particleswarm(fun, 2, [], ub);");
    }

    #[test]
    fn optimize_app_options_only_when_changed_from_default() {
        let ws = Snapshot::default();
        let mut host = Host::new(Optimize::default());
        let options = |app: &mut Optimize| -> Option<String> {
            app.run();
            let script = app.script(&ws).unwrap();
            script
                .lines()
                .find(|l| l.starts_with("options = "))
                .map(str::to_owned)
        };
        let app = &mut host.app;
        // Defaults typed out are still defaults.
        app.spec.options.set(Opt::MaxIterations, "400");
        app.spec.options.set(Opt::OptimalityTolerance, "1e-6");
        app.spec.options.set(Opt::MaxFunctionEvaluations, "200");
        app.spec.options.display = Some(DisplayLevel::Final);
        assert_eq!(options(app), None);
        assert!(app.script(&ws).unwrap().contains("fminunc(fun, x0);"));
        app.spec.options.set(Opt::MaxIterations, "5");
        app.spec.options.set(Opt::MaxFunctionEvaluations, "300");
        app.spec.options.set(Opt::StepTolerance, "1e-8");
        app.spec.options.display = Some(DisplayLevel::Iter);
        assert_eq!(
            options(app).unwrap(),
            "options = optimoptions('fminunc', 'Display', 'iter', 'MaxIterations', 5, \
             'MaxFunctionEvaluations', 300, 'StepTolerance', 1e-8);"
        );
        assert!(
            app.script(&ws)
                .unwrap()
                .contains("fminunc(fun, x0, options);")
        );
        assert!(app.exported(&ws).contains(&"options".to_owned()));
        // The options take effect: five iterations end at the limit.
        let script = solved_script(&mut host, &ws, "");
        assert!(script.contains("options"));
        let r = shown(&host);
        assert_eq!((r.exitflag, r.iterations), (0., Some(5.)));
        // Options a solver does not have are left out.
        let app = &mut host.app;
        app.spec.options.set(Opt::ConstraintTolerance, "1e-3");
        assert!(!options(app).unwrap().contains("ConstraintTolerance"));
        // fsolve's algorithm: the default is dropped, the other kept.
        app.spec.options = Default::default();
        app.spec.problem = Problem::Equations;
        app.spec.system = true;
        app.spec.objective.expression = "[x(1) - 1; x(2) + 1]".into();
        app.spec.options.algorithm = Some("trust-region-dogleg");
        assert_eq!(options(app), None);
        app.spec.options.algorithm = Some("levenberg-marquardt");
        assert_eq!(
            options(app).unwrap(),
            "options = optimoptions('fsolve', 'Algorithm', 'levenberg-marquardt');"
        );
        // fminsearch, fminbnd and fzero take no options in MatRust.
        app.spec = Default::default();
        app.spec.solver = Some(Solver::Fminsearch);
        app.spec.options.set(Opt::MaxIterations, "5");
        app.spec.options.display = Some(DisplayLevel::Iter);
        assert_eq!(options(app), None);
        // Global solvers: per-variable defaults.
        app.spec = Default::default();
        app.spec.problem = Problem::Global;
        app.spec.solver = Some(Solver::Patternsearch);
        app.spec.options.set(Opt::MaxIterations, "200");
        assert_eq!(options(app), None, "100 x 2 variables");
        app.spec.options.set(Opt::MaxIterations, "Inf");
        assert_eq!(
            options(app).unwrap(),
            "options = optimoptions('patternsearch', 'MaxIterations', Inf);"
        );
        // A value that is not a number stops before running.
        app.spec.options.set(Opt::MaxIterations, "lots");
        app.run();
        let error = app.script(&ws).unwrap_err();
        assert!(
            error.contains("Max iterations must be a whole number"),
            "{error}"
        );
    }

    #[test]
    fn optimize_app_objective_sources_and_workspace_data() {
        let ws = workspace(
            "rosen = @(x) (x(1)-1)^2 + 100*(x(2)-x(1)^2)^2; a = 3; start = [-1 2]; x0 = 5;",
        );
        let setup = "rosen = @(x) (x(1)-1)^2 + 100*(x(2)-x(1)^2)^2; a = 3; start = [-1 2]; x0 = 5;";
        let mut host = Host::new(Optimize::default());
        // A workspace handle and a workspace start point: no definitions.
        host.app.spec.objective.source = Source::Workspace;
        host.app.spec.objective.variable = "rosen".into();
        host.app.spec.set(Slot::X0, "start");
        let script = solved_script(&mut host, &ws, setup);
        assert!(
            script.contains("fminunc(rosen, start);") && !script.contains("fun ="),
            "{script}"
        );
        assert_eq!(host.app.inputs(), ["start", "rosen"]);
        // A typed expression reading a workspace variable; x0 is taken, so
        // the typed start point is x01.
        host.app.spec.objective.source = Source::Expression;
        host.app.spec.objective.expression = "a*(x(1)-1)^2 + (x(2)+2)^2".into();
        host.app.spec.set(Slot::X0, "[0 0]");
        let script = solved_script(&mut host, &ws, setup);
        assert!(
            script.contains("x01 = [0 0];") && script.contains("fminunc(fun, x01);"),
            "{script}"
        );
        assert_eq!(host.app.inputs(), ["a"]);
        let r = shown(&host);
        assert!(
            close(r.x[0], 1., 1e-4) && close(r.x[1], -2., 1e-4),
            "{:?}",
            r.x
        );
        // A function file in the desktop's folder.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("bowl.m"),
            "function y = bowl(x)\ny = (x(1) - 3)^2 + (x(2) - 4)^2;\nend\n",
        )
        .unwrap();
        host.app.spec.objective.source = Source::File;
        host.app.spec.objective.file = "bowl".into();
        host.app.run();
        host.app.prepare(&ws);
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("fminunc(@bowl, x01);"), "{script}");
        let mut engine = matrust::Engine::default();
        engine.set_file_directory(dir.path()).unwrap();
        let outputs = apps::run(&mut engine, host.request(&ws).unwrap()).unwrap();
        host.app.receive(&outputs).unwrap();
        let r = shown(&host);
        assert!(
            close(r.x[0], 3., 1e-5) && close(r.x[1], 4., 1e-5),
            "{:?}",
            r.x
        );
        // The host's runner starts in the folder it is given.
        let ctx = egui::Context::default();
        host.app.spec.set(Slot::X0, "[1 1]");
        host.app.run();
        host.set_folder(Some(dir.path()));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            host.refresh(&ctx, &ws);
            if !host.running() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{:?}",
            host.status()
        );
        let r = shown(&host);
        assert!(
            close(r.x[0], 3., 1e-5) && close(r.x[1], 4., 1e-5),
            "{:?}",
            r.x
        );
    }

    #[test]
    fn optimize_app_errors_size_mismatch_and_failing_objective() {
        let ws = Snapshot::default();
        let mut host = Host::new(Optimize::default());
        // Nothing runs before Run.
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("press Run"), "{error}");
        solve(&mut host, &ws).unwrap();
        let good = shown(&host).x.clone();
        // Size mismatches never reach the engine.
        host.app.spec.constraints.bounds = true;
        host.app.spec.set(Slot::Lb, "[0 0 0]");
        host.app.run();
        let error = host.request(&ws).err().unwrap();
        assert!(
            error.contains("Lower bounds lb has 3 values but x0 has 2 variables"),
            "{error}"
        );
        host.app.spec.constraints.bounds = false;
        host.app.spec.constraints.inequalities = true;
        host.app.spec.set(Slot::A, "[1 1 1]");
        host.app.spec.set(Slot::B, "1");
        host.app.run();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("A has 3 columns but x0 has 2"), "{error}");
        host.app.spec.set(Slot::A, "[1 1; 1 0]");
        host.app.run();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("b has 1 values but A has 2 rows"), "{error}");
        host.app.spec.set(Slot::B, "");
        host.app.run();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("turn Linear inequalities off"), "{error}");
        host.app.spec.constraints.inequalities = false;
        // A workspace variable of the wrong kind.
        let text_ws = workspace("s = 'abc';");
        host.app.spec.set(Slot::X0, "s");
        host.app.run();
        let error = host.app.script(&text_ws).unwrap_err();
        assert!(error.contains("not a real numeric array"), "{error}");
        // An objective that errors: the engine's message, the last result stays.
        host.app.spec.set(Slot::X0, "[-1 2]");
        host.app.spec.objective.expression = "x(3)^2".into();
        let error = solve(&mut host, &ws).unwrap_err();
        assert!(error.to_lowercase().contains("index"), "{error}");
        assert_eq!(shown(&host).x, good, "the last good result stays");
        // Output names must be names and must differ.
        host.app.spec.objective.expression = "sum(x.^2)".into();
        host.app.names[0] = "res".into();
        host.app.names[1] = "res".into();
        host.app.edited = [true, true, false, false];
        host.app.run();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("must differ"), "{error}");
        host.app.names[1] = "2bad".into();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("not a valid variable name"), "{error}");
        // A default name steps aside for an edited one.
        host.app.names[0] = "objectiveValue".into();
        host.app.edited = [true, false, false, false];
        let names = host.app.names_in_use(&ws);
        assert_eq!(names[..2], ["objectiveValue", "objectiveValue1"]);
        // An output name may not overwrite an input of the problem.
        let data_ws = workspace("start = [-1 2];");
        host.app.spec.set(Slot::X0, "start");
        host.app.names[0] = "start".into();
        host.app.run();
        let error = host.app.script(&data_ws).unwrap_err();
        assert!(error.contains("is an input of the problem"), "{error}");
        host.app.spec.set(Slot::X0, "[-1 2]");
        host.app.edited = [false; 4];
        // Through the threaded runner the status line carries the engine error.
        let ctx = egui::Context::default();
        host.app.spec.objective.expression = "undefined_thing(x)".into();
        host.app.run();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            host.refresh(&ctx, &ws);
            if !host.running() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let (text, status) = host.status();
        assert_eq!(status, toolkits::AppStatus::Error);
        assert!(text.contains("undefined_thing"), "{text}");
        assert_eq!(shown(&host).x, good);
    }

    #[test]
    fn optimize_app_stop_cancels_the_running_solve() {
        let dir = tempfile::tempdir().unwrap();
        // Each evaluation waits 50 ms: the solve would take many seconds.
        std::fs::write(
            dir.path().join("slowbowl.m"),
            "function y = slowbowl(x)\npause(0.05);\ny = sum((x - 3).^2);\nend\n",
        )
        .unwrap();
        let ws = Snapshot::default();
        let ctx = egui::Context::default();
        let mut host = Host::new(Optimize::default());
        host.set_folder(Some(dir.path()));
        // A finished solve first: Stop keeps it on screen.
        solve(&mut host, &ws).unwrap();
        let good = shown(&host).x.clone();
        host.app.spec.objective.source = Source::File;
        host.app.spec.objective.file = "slowbowl".into();
        host.app.run();
        host.refresh(&ctx, &ws);
        std::thread::sleep(Duration::from_millis(300));
        host.refresh(&ctx, &ws);
        assert!(host.running(), "{:?}", host.status());
        host.app.running = host.running();
        let start = Instant::now();
        host.app.stop();
        assert!(host.app.stopped && host.app.applied.is_none());
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            host.refresh(&ctx, &ws);
            if !host.running() {
                break;
            }
            assert!(Instant::now() < deadline, "the solve was not cancelled");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
        let (text, status) = host.status();
        assert_eq!(status, toolkits::AppStatus::Error);
        assert!(text.contains("stopped"), "{text}");
        assert_eq!(shown(&host).x, good);
        // Run again solves (and the stop note goes).
        host.app.spec.objective.source = Source::Expression;
        host.app.run();
        assert!(!host.app.stopped);
        assert!(host.app.script(&ws).is_ok());
    }

    #[test]
    fn optimize_app_private_runs_have_the_desktop_limits() {
        // An objective with a loop: the solve, the recorded solve and the map
        // run it about 1700 times, over two million statements in all. The
        // workspace runs that without a limit, so the app's run must too.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("loopy.m"),
            "function y = loopy(x)\ny = 0;\nfor k = 1:700\ny = y + 1;\nend\n\
             y = (x(1) - 1)^2 + (x(2) - 2)^2 + 0 * y;\nend\n",
        )
        .unwrap();
        let ws = Snapshot::default();
        let ctx = egui::Context::default();
        let mut host = Host::new(Optimize::default());
        host.set_folder(Some(dir.path()));
        host.app.spec.objective.source = Source::File;
        host.app.spec.objective.file = "loopy".into();
        host.app.run();
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            host.refresh(&ctx, &ws);
            if !host.running() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{:?}",
            host.status()
        );
        let r = shown(&host);
        assert!(
            close(r.x[0], 1., 1e-4) && close(r.x[1], 2., 1e-4),
            "{:?}",
            r.x
        );
        assert!(
            r.map
                .as_ref()
                .is_some_and(|m| m.values.iter().all(|v| v.is_finite()))
        );
    }

    #[test]
    fn optimize_app_map_fits_a_time_budget() {
        // 10 ms per evaluation: a 41 x 41 grid would take 17 s, so the map
        // takes a coarser grid (~17 x 17) that fits its few seconds; the
        // margin to the 11-point floor absorbs parallel-test CPU contention.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("slowish.m"),
            "function y = slowish(x)\npause(0.01);\ny = (x(1) - 1)^2 + (x(2) - 2)^2;\nend\n",
        )
        .unwrap();
        let ws = Snapshot::default();
        let mut host = Host::new(Optimize::default());
        host.app.spec.objective.source = Source::File;
        host.app.spec.objective.file = "slowish".into();
        host.app.run();
        host.app.prepare(&ws);
        let mut engine = matrust::Engine::default();
        engine.set_file_directory(dir.path()).unwrap();
        let start = Instant::now();
        let outputs = apps::run(&mut engine, host.request(&ws).unwrap()).unwrap();
        host.app.receive(&outputs).unwrap();
        let map = shown(&host).map.as_ref().expect("a coarser map");
        assert!(
            (11..apps::optimize_app::GRID).contains(&map.x.len()),
            "{}",
            map.x.len()
        );
        assert_eq!(map.values.len(), map.x.len() * map.y.len());
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "{:?}",
            start.elapsed()
        );
    }

    fn open_optimize(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) -> egui::FullOutput {
        app.toolkits.open_tool(Tool::App(AppKind::Optimize));
        frame_at(app, ctx, size, vec![]);
        app.toolkits.apps.optimize.app.run();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let output = frame_at(app, ctx, size, vec![]);
            let host = &app.toolkits.apps.optimize;
            if host.app.result.is_some() && !host.running() {
                return frame_at(app, ctx, size, vec![]);
            }
            assert!(Instant::now() < deadline, "{:?}", host.status());
            drop(output);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn painted(output: &egui::FullOutput, label: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, label).is_some())
    }

    #[test]
    fn optimize_app_tab_renders_inputs_views_and_output() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        let output = open_optimize(&mut app, &ctx, size);
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(Tool::App(AppKind::Optimize))
        );
        for label in [
            "INPUTS",
            "SOLVER",
            "OUTPUT",
            "Run",
            "Stop",
            "Problem",
            "Minimize a function",
            "Constraints",
            "fminunc (recommended)",
            "Progress",
            "Objective map",
            "Solver",
            "Exit flag",
            "Export to workspace",
            "Generate script",
            "Solution name",
        ] {
            assert!(painted(&output, label), "{label} not painted");
        }
        let x = apps::optimize_app::vector_text(
            &app.toolkits.apps.optimize.app.result.as_ref().unwrap().x,
        );
        assert!(painted(&output, &x), "x {x}");
        let (text, status) = app
            .toolkits
            .app_status(Tool::App(AppKind::Optimize), false)
            .unwrap();
        assert_eq!(status, toolkits::AppStatus::Success);
        assert!(text.starts_with("fminunc"), "{text}");
        // The map renders.
        click_label_at(&mut app, &ctx, size, "Objective map");
        let output = frame_at(&mut app, &ctx, size, vec![]);
        assert_eq!(app.toolkits.apps.optimize.app.view, View::Map);
        assert!(painted(&output, "Objective map"));
        // A narrow host stacks OUTPUT under the main view.
        let output = frame_at(&mut app, &ctx, egui::vec2(900., 700.), vec![]);
        assert!(painted(&output, "OUTPUT"));
        // Closing keeps the inputs and the result for the next open.
        app.close_tool_tab(Tool::App(AppKind::Optimize));
        assert!(app.toolkits.tabs().next().is_none());
        assert!(app.toolkits.apps.optimize.app.result.is_some());
    }

    #[test]
    fn optimize_app_export_and_generate_script_do_not_clobber() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("solved.m");
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run("solution = 7; fun = 3;".into());
        finish(&mut app);
        open_optimize(&mut app, &ctx, size);
        let shown_x = app
            .toolkits
            .apps
            .optimize
            .app
            .result
            .as_ref()
            .unwrap()
            .x
            .clone();
        let script = {
            let ws = &app.toolkits.apps.workspace;
            app.toolkits.apps.optimize.app.script(ws).unwrap()
        };
        assert!(
            script.contains("fun1 = @(x)")
                && script
                    .contains("[solution1, objectiveValue, exitflag, output] = fminunc(fun1, x0);"),
            "{script}"
        );
        click_label_at(&mut app, &ctx, size, "Export to workspace");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        let value = |name: &str| match &app.variables[name] {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            other => panic!("{name}: {}", other.class()),
        };
        assert_eq!(value("solution"), [7.], "solution is kept");
        assert_eq!(value("fun"), [3.], "fun is kept");
        assert_eq!(value("solution1"), shown_x, "{}", app.status);
        assert_eq!(value("exitflag"), [1.]);
        assert!(matches!(app.variables["output"], matrust::Array::Struct(_)));
        assert!(matches!(app.variables["fun1"], matrust::Array::Handle(_)));
        assert!(
            app.toolkits.apps.optimize.message.contains("solution1"),
            "{}",
            app.toolkits.apps.optimize.message
        );
        // Every exported name is taken now: the next defaults move on.
        frame_at(&mut app, &ctx, size, vec![]);
        let ws = &app.toolkits.apps.workspace;
        let next = app.toolkits.apps.optimize.app.script(ws).unwrap();
        assert!(
            next.contains("[solution2, objectiveValue1, exitflag1, output1] = fminunc(fun2, x01);"),
            "{next}"
        );
        // Generate script: a header and the same script, which runs.
        let script = next;
        // Wait for the rerun the new names started.
        let deadline = Instant::now() + Duration::from_secs(60);
        while app.toolkits.apps.optimize.running() {
            frame_at(&mut app, &ctx, size, vec![]);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        app.toolkits.apps.test_save_path = Some(path.clone());
        click_label_at(&mut app, &ctx, size, "Generate script");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.starts_with("% Optimize (MatRust app), generated "),
            "{saved}"
        );
        assert!(saved.ends_with(&script), "{saved}");
        let mut engine = matrust::Engine::default();
        engine.execute(&saved).unwrap();
        assert_eq!(numbers(&engine, "solution2"), shown_x);
    }

    #[test]
    fn optimize_app_rejects_statement_injection_in_expression_fields() {
        let ws = Snapshot::default();
        for payload in [
            "sum(x.^2); injected = 123",
            "sum(x.^2), clear victim",
            "sum(x.^2) % swallowed",
        ] {
            let mut app = Optimize::default();
            app.spec.objective.expression = payload.into();
            app.run();
            assert!(app.script(&ws).is_err(), "accepted objective {payload}");
        }
        for slot in [
            Slot::X0,
            Slot::Lb,
            Slot::Ub,
            Slot::A,
            Slot::B,
            Slot::Aeq,
            Slot::Beq,
        ] {
            let mut app = Optimize::default();
            app.spec.constraints.bounds = true;
            app.spec.constraints.inequalities = true;
            app.spec.constraints.equalities = true;
            app.spec.set(Slot::Lb, "[-5 -5]");
            app.spec.set(Slot::Ub, "[5 5]");
            app.spec.set(Slot::A, "[1 1]");
            app.spec.set(Slot::B, "3");
            app.spec.set(Slot::Aeq, "[1 -1]");
            app.spec.set(Slot::Beq, "0");
            app.spec.set(slot, "[1 2]; injected = 123");
            app.run();
            assert!(app.script(&ws).is_err(), "accepted {slot:?}");
        }
        for (problem, slot) in [
            (Problem::Linear, Slot::F),
            (Problem::MixedInteger, Slot::Intcon),
            (Problem::Quadratic, Slot::H),
            (Problem::CurveFit, Slot::Xdata),
            (Problem::CurveFit, Slot::Ydata),
        ] {
            let mut app = Optimize::default();
            app.spec.problem = problem;
            app.spec.set(Slot::F, "[1; 2]");
            app.spec.set(Slot::H, "[1 0; 0 1]");
            app.spec.set(Slot::Intcon, "[1 2]");
            app.spec.set(Slot::Xdata, "0:3");
            app.spec.set(Slot::Ydata, "1:4");
            app.run();
            app.script(&ws).unwrap();
            app.spec.set(slot, "[1 2]; injected = 123");
            app.run();
            let error = app.script(&ws).expect_err("statement escape");
            assert!(error.contains("one expression"), "{slot:?}: {error}");
        }
        for equality in [false, true] {
            let mut app = Optimize::default();
            app.spec.constraints.nonlinear = true;
            if equality {
                app.spec.ceq = "x(1); injected = 123".into();
            } else {
                app.spec.nonlcon.expression = "x(1); injected = 123".into();
            }
            app.run();
            assert!(app.script(&ws).is_err(), "accepted nonlinear constraint");
        }
        let mut app = Optimize::default();
        app.spec
            .options
            .set(Opt::MaxIterations, "5; injected = 123");
        app.run();
        assert!(app.script(&ws).is_err());
    }

    #[test]
    fn optimize_app_population_solvers_emit_only_public_options() {
        let ws = Snapshot::default();
        for solver in [Solver::Ga, Solver::Particleswarm] {
            let mut app = Optimize::default();
            app.spec.problem = Problem::Global;
            app.spec.solver = Some(solver);
            app.spec.options.set(Opt::MaxFunctionEvaluations, "123");
            app.spec.options.set(Opt::MaxGenerations, "3");
            app.spec.options.set(Opt::MaxIterations, "4");
            app.spec.options.set(Opt::FunctionTolerance, "1e-9");
            app.run();
            let script = app.script(&ws).unwrap();
            assert!(!solver.options().contains(&Opt::MaxFunctionEvaluations));
            assert!(!script.contains("MaxFunctionEvaluations"), "{script}");
            assert!(script.contains("'FunctionTolerance', 1e-9"), "{script}");
            let count = if solver == Solver::Ga {
                "'MaxGenerations', 3"
            } else {
                "'MaxIterations', 4"
            };
            assert!(script.contains(count), "{script}");
        }
    }

    #[test]
    fn optimize_app_folder_change_changes_request_key() {
        let ws = Snapshot::default();
        let mut host = Host::new(Optimize::default());
        host.app.run();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        host.set_folder(Some(a.path()));
        let first = host.request(&ws).unwrap().key;
        host.set_folder(Some(b.path()));
        assert_ne!(first, host.request(&ws).unwrap().key);
    }
}
