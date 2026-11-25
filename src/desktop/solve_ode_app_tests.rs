// Solve ODE app (apps/solve_ode_app.rs): the generated script run in a fresh
// Engine gives the y(tf) the app shows (decay, van der Pol, the y'' helper,
// a falling ball's event, output times, ode15i), typed-equation parsing,
// option filtering, plot calls, Run/Stop, export without clobbering, errors,
// the saved script and a rendered tab.
mod solve_ode_app_tests {
    use super::*;
    use apps::solve_ode_app::{
        COUNT_LIMIT, Direction, Events, Out, Plot, STIFF_STEPS, SolveOde, Solver, Source, Spec,
        parse_system,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    /// One private run, synchronously, delivered to the app.
    fn solve(host: &mut Host<SolveOde>, ws: &Snapshot) -> Result<(), String> {
        host.app.prepare(ws);
        let request = host.request(ws)?;
        let outputs = apps::run(&mut matrust::Engine::default(), request)?;
        host.app.receive(&outputs)
    }
    fn solution(host: &Host<SolveOde>) -> &apps::solve_ode_app::Solution {
        host.app.result.as_ref().expect("a solution")
    }
    fn last(host: &Host<SolveOde>) -> Vec<f64> {
        solution(host).last()
    }
    /// Runs `setup` and `script` in a fresh Engine and returns `expr`.
    fn run_script(setup: &str, script: &str, expr: &str) -> Vec<f64> {
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        engine
            .execute(script)
            .unwrap_or_else(|e| panic!("{script}\n{e}"));
        engine.execute(&format!("check_ = {expr};")).unwrap();
        match engine.array("check_") {
            Some(matrust::Array::Numeric(m)) => m.data().to_vec(),
            other => panic!("{expr}: {other:?}"),
        }
    }
    fn typed(
        equations: &str,
        parameters: &[(&str, &str)],
        y0: &str,
        tspan: &str,
    ) -> Host<SolveOde> {
        let mut host = Host::new(SolveOde::default());
        let spec = &mut host.app.spec;
        spec.equations = equations.into();
        spec.parameters = parameters
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect();
        spec.y0 = y0.into();
        spec.tspan = tspan.into();
        host
    }
    fn workspace(setup: &str) -> Snapshot {
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }

    #[test]
    fn solve_ode_app_exponential_decay_matches_the_exact_solution() {
        let ws = Snapshot::default();
        let mut host = typed("-k*y1", &[("k", "0.5")], "3", "[0 2]");
        solve(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert_eq!(
            script,
            "k = 0.5;\nf = @(t, y) -k*y(1);\n[t, y] = ode45(f, [0 2], 3);\n\
             plot(t, y)\nxlabel('t')\nlegend('y1')\n"
        );
        let y = last(&host)[0];
        let exact = 3. * (-1f64).exp();
        assert!((y - exact).abs() < 1e-6 * exact, "{y} vs {exact}");
        // The script in a fresh Engine gives the y(tf) the app shows.
        assert_eq!(run_script("", &script, "y(end, :)"), [y]);
        let r = solution(&host);
        assert_eq!(r.solver, Solver::Ode45);
        assert_eq!((r.t[0], *r.t.last().unwrap()), (0., 2.));
        // ode45 evaluates f six times per attempted step, plus once at t0.
        let [steps, failed, evaluations] = r.stats;
        assert!(steps > 0.);
        assert_eq!(evaluations, 6. * (steps + failed) + 1.);
        assert!(r.wall.is_finite() && r.wall >= 0.);
        // The Output block shows those counts and y(tf).
        let rows = host.app.results();
        assert_eq!(rows[0], ("Solver", "ode45".to_owned()));
        assert_eq!(rows[1], ("Steps", apps::short(steps)));
        assert_eq!(rows[3], ("Function evaluations", apps::short(evaluations)));
        assert!(host.app.summary().unwrap().starts_with("ode45 · y(2) = ["));
        assert_eq!(host.app.exported(&ws), ["t", "y", "k", "f"]);
    }

    #[test]
    fn solve_ode_app_van_der_pol_nonstiff_and_stiff() {
        let ws = Snapshot::default();
        // The default inputs are van der Pol with mu = 1.
        let mut host = Host::new(SolveOde::default());
        host.app.spec.rel_tol = "1e-8".into();
        host.app.spec.abs_tol = "1e-10".into();
        solve(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.starts_with(
                "mu = 1;\nf = @(t, y) [y(2); mu*(1 - y(1)^2)*y(2) - y(1)];\n\
                 opts = odeset('RelTol', 1e-8, 'AbsTol', 1e-10);\n\
                 [t, y] = ode45(f, [0 20], [2 0], opts);\n"
            ),
            "{script}"
        );
        // SciPy Radau at rtol 1e-11 (tool/refs/solve_ode_app/vdp_reference.py).
        let y = last(&host);
        assert!((y[0] - 2.00814976217).abs() < 1e-6, "{y:?}");
        assert!((y[1] + 0.0425088752732).abs() < 1e-5, "{y:?}");
        assert_eq!(run_script("", &script, "y(end, :)"), y);
        assert!(!solution(&host).stiff_hint());
        // mu = 1000 is stiff: Auto with the box ticked uses ode15s.
        host.app.spec.parameters = vec![("mu".into(), "1000".into())];
        host.app.spec.tspan = "[0 3000]".into();
        host.app.spec.stiff = true;
        host.app.spec.rel_tol = "1e-6".into();
        host.app.spec.abs_tol = "1e-8".into();
        solve(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("[t, y] = ode15s(f, [0 3000], [2 0], opts);"),
            "{script}"
        );
        let y = last(&host);
        assert!((y[0] + 1.51060693675).abs() < 5e-4, "{y:?}");
        assert!((y[1] - 0.00117838000073).abs() < 1e-5, "{y:?}");
        assert_eq!(run_script("", &script, "y(end, :)"), y);
        assert_eq!(solution(&host).solver, Solver::Ode15s);
        assert!(!solution(&host).stiff_hint());
        // ode45 on the stiff problem: the step count explodes and the app
        // says so.
        host.app.spec.stiff = false;
        host.app.spec.tspan = "[0 10]".into();
        host.app.spec.rel_tol.clear();
        host.app.spec.abs_tol.clear();
        solve(&mut host, &ws).unwrap();
        let r = solution(&host);
        assert_eq!(r.solver, Solver::Ode45);
        // By design a first solve slower than COUNT_LIMIT skips the counting
        // re-solve (stats NaN); a loaded Windows gate machine hits that branch.
        assert!(
            r.stats[0] > STIFF_STEPS || (r.stats[0].is_nan() && r.wall > COUNT_LIMIT),
            "{:?} wall {}",
            r.stats,
            r.wall
        );
        assert!(r.stiff_hint());
        assert!(host.app.summary().unwrap().contains("may be stiff"));
    }

    #[test]
    fn solve_ode_app_second_order_helper_becomes_a_first_order_system() {
        let system = parse_system("x'' = -2*z*w*x' - w^2*x", false).unwrap();
        assert_eq!(system.rows, ["y(2)", "-2*z*w*y(2) - w^2*y(1)"]);
        assert_eq!(system.labels, ["x", "x'"]);
        assert_eq!(system.converted, Some(("x".into(), 2)));
        let third = parse_system("u''' = -u' % a comment", false).unwrap();
        assert_eq!(third.rows, ["y(2)", "y(3)", "-y(2)"]);
        let error = parse_system("x'' = -x''", false).unwrap_err();
        assert!(error.contains("up to x'"), "{error}");
        // An undamped oscillator: x(pi) = cos(2 pi) = 1, x'(pi) = 0.
        let ws = Snapshot::default();
        let mut host = typed("x'' = -w^2*x", &[("w", "2")], "[1 0]", "[0 pi]");
        host.app.spec.rel_tol = "1e-8".into();
        host.app.spec.abs_tol = "1e-10".into();
        solve(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.starts_with(
                "% x'' = -w^2*x as a first-order system: y(1) = x, y(2) = x'\n\
                 w = 2;\nf = @(t, y) [y(2); -w^2*y(1)];\n"
            ),
            "{script}"
        );
        assert!(script.contains("legend('x', 'x''')\n"), "{script}");
        let y = last(&host);
        assert!((y[0] - 1.).abs() < 1e-6 && y[1].abs() < 1e-5, "{y:?}");
        assert_eq!(solution(&host).labels, ["x", "x'"]);
        assert_eq!(run_script("", &script, "y(end, :)"), y);
        // The preview (Script tab) is the same text.
        host.app.prepare(&ws);
        assert_eq!(host.app.build(&ws).unwrap(), script);
    }

    #[test]
    fn solve_ode_app_typed_equations_parse_like_matlab() {
        // Left sides are optional and may come in any order.
        let system = parse_system("y2' = -y1\ny1' = y2;", false).unwrap();
        assert_eq!(system.rows, ["y(2)", "-y(1)"]);
        // Strings, transposes, fields, exponents and comments stay as typed.
        let system = parse_system("1e-3*y1 + s.y1 - y1' + numel('y1') % y2", false).unwrap();
        assert_eq!(system.rows, ["1e-3*y(1) + s.y1 - y(1)' + numel('y1')"]);
        // A row that would split inside brackets gets parentheses.
        let system = parse_system("y1 -y2\ny1", false).unwrap();
        assert_eq!(system.body(), "[(y(1) -y(2)); y(1)]");
        for (text, expected) in [
            ("y1\ny3", "y3 is not one of y1 to y2"),
            ("y1' = 1\ny1' = 2", "y1' is defined twice"),
            ("x' = v\nv' = -x", "the left side must be y1'"),
            ("y2\n-y1 + * 2", "equation 2 does not parse"),
            ("y1 + 'abc", "a quote is not closed"),
            ("", "type the equations"),
        ] {
            let error = parse_system(text, false).unwrap_err();
            assert!(error.contains(expected), "{text}: {error}");
        }
        // ode15i: residuals in y and yp (or y'), `a = b` means a - b.
        let system = parse_system("yp1 + 2*y1\ny2' = y1", true).unwrap();
        assert_eq!(system.rows, ["yp(1) + 2*y(1)", "yp(2) - (y(1))"]);
    }

    #[test]
    fn solve_ode_app_ball_hits_the_ground_event() {
        let ws = Snapshot::default();
        let mut host = typed("y2\n-g", &[("g", "9.81")], "[10 0]", "[0 5]");
        host.app.spec.events = Events::Condition;
        host.app.spec.event_value = "y1".into();
        host.app.spec.direction = Direction::Falling;
        host.app.spec.terminal = true;
        solve(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains(
                "opts = odeset('Events', @zeroCrossing);\n\
                 [t, y, te, ye, ie] = ode113(f, [0 5], [10 0], opts);\n"
            ),
            "{script}"
        );
        assert!(
            script.ends_with(
                "\nfunction [value, isterminal, direction] = zeroCrossing(t, y)\n\
                 % An event is a zero of value.\nvalue = y(1);\nisterminal = 1;\n\
                 direction = -1;\nend\n"
            ),
            "{script}"
        );
        assert!(script.contains("plot(repmat(te, 1, size(ye, 2)), ye, 'o')"));
        // Auto uses ode113: MatRust's ode45 has no event location yet.
        let r = solution(&host);
        assert_eq!(r.solver, Solver::Ode113);
        let exact = (2. * 10. / 9.81_f64).sqrt();
        assert_eq!(r.events(), 1);
        assert!((r.te[0] - exact).abs() < 1e-3, "{} vs {exact}", r.te[0]);
        assert!(r.event(0, 0).abs() < 1e-6 && r.event(0, 1) < 0.);
        assert_eq!(r.ie, [1.]);
        // A terminal event ends the solve there.
        assert!((r.t.last().unwrap() - r.te[0]).abs() < 1e-12);
        let shown = [r.te[0], r.event(0, 0), r.event(0, 1), r.ie[0]];
        assert_eq!(run_script("", &script, "[te ye ie]"), shown);
        assert_eq!(run_script("", &script, "y(end, :)"), last(&host));
        // The export wrapper keeps the local function outside its try block.
        let exported = host.app.exported(&ws);
        assert_eq!(exported, ["t", "y", "te", "ye", "ie", "g", "f", "opts"]);
        let wrapped = apps::all_or_nothing(&script, &exported);
        assert!(wrapped.ends_with(&format!(
            "clear mrexp_had mrexp_old\n{}",
            apps::split_local_functions(&script).1
        )));
        assert_eq!(run_script("", &wrapped, "te"), [r.te[0]]);
        // A parameter in the condition reaches the local function.
        host.app.spec.parameters.push(("h".into(), "2".into()));
        host.app.spec.event_value = "y1 - h".into();
        solve(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("odeset('Events', @(t, y) zeroCrossing(t, y, h))"),
            "{script}"
        );
        assert!(script.contains("= zeroCrossing(t, y, h)\n"), "{script}");
        let exact = (2. * 8. / 9.81_f64).sqrt();
        assert!((solution(&host).te[0] - exact).abs() < 1e-3);
        // Structure output: the events are sol.xe, sol.ye and sol.ie.
        host.app.structure = true;
        solve(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("sol = ode113(f, [0 5], [10 0], opts);\nplot(sol.x, sol.y)\n"),
            "{script}"
        );
        assert!(script.contains("plot(repmat(sol.xe, size(sol.ye, 1), 1), sol.ye, 'o')"));
        assert_eq!(run_script("", &script, "sol.xe"), [solution(&host).te[0]]);
        // An explicit ode45 locates the same event (fix 19b1).
        host.app.spec.solver = Solver::Ode45;
        solve(&mut host, &ws).unwrap();
        assert!((solution(&host).te[0] - exact).abs() < 1e-3);
    }

    #[test]
    fn solve_ode_app_function_files_and_handles_from_the_current_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("fall.m"),
            "function dydt = fall(t, y)\ndydt = [y(2); -9.81];\nend\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("ground.m"),
            "function [v, term, dir] = ground(t, y)\nv = y(1); term = 1; dir = -1;\nend\n",
        )
        .unwrap();
        let mut ws = workspace("decay = @(t, y) -2*y; ev = @ground; x = 1;");
        ws.set_folder(Some(dir.path().to_path_buf()));
        let mut host = Host::new(SolveOde::default());
        host.app.spec.source = Source::Function;
        host.app.spec.function = "fall".into();
        host.app.spec.y0 = "[10 0]".into();
        host.app.spec.tspan = "[0 5]".into();
        host.app.spec.events = Events::Function;
        host.app.spec.event_function = "ev".into();
        solve(&mut host, &ws).unwrap();
        assert_eq!(host.app.inputs(), ["ev"]);
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.starts_with(
                "opts = odeset('Events', ev);\n\
                 [t, y, te, ye, ie] = ode113(@fall, [0 5], [10 0], opts);\n"
            ),
            "{script}"
        );
        let exact = (2. * 10. / 9.81_f64).sqrt();
        assert!((solution(&host).te[0] - exact).abs() < 1e-3);
        // The private run starts in the current folder; the script does not.
        let request = host.request(&ws).unwrap();
        assert!(request.code.starts_with("try\ncd('"), "{}", request.code);
        assert!(!script.contains("cd("));
        // A workspace handle is used by name; y0 must match its output.
        host.app.spec.function = "decay".into();
        host.app.spec.events = Events::None;
        host.app.spec.y0 = "1".into();
        solve(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.starts_with(
            "% decay is the workspace function handle f(t, y).\n[t, y] = ode45(decay, [0 5], 1);\n"
        ));
        assert!((last(&host)[0] - (-10f64).exp()).abs() < 1e-6);
        assert_eq!(
            run_script("decay = @(t, y) -2*y;", &script, "y(end)"),
            last(&host)
        );
        // An anonymous function typed in the field.
        host.app.spec.function = "@(t, y) -x*y".into();
        solve(&mut host, &ws).unwrap();
        assert_eq!(host.app.inputs(), ["x"]);
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .starts_with("f = @(t, y) -x*y;\n")
        );
        // Not a handle, or y0 of the wrong length: the engine says why.
        host.app.spec.function = "x".into();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("x is not a function handle"), "{error}");
        host.app.spec.function = "fall".into();
        host.app.spec.y0 = "[1 2 3]".into();
        let error = solve(&mut host, &ws).unwrap_err();
        assert!(error.contains("initial conditions"), "{error}");
    }

    #[test]
    fn solve_ode_app_output_times_vector() {
        let ws = Snapshot::default();
        for tspan in ["0:0.5:2", "[0 0.5 1 1.5 2]", "0 0.5 1 1.5 2"] {
            let mut host = typed("-y1", &[], "1", tspan);
            host.app.spec.rel_tol = "1e-8".into();
            host.app.spec.abs_tol = "1e-10".into();
            solve(&mut host, &ws).unwrap_or_else(|e| panic!("{tspan}: {e}"));
            let r = solution(&host);
            assert_eq!(r.t, [0., 0.5, 1., 1.5, 2.], "{tspan}");
            for (t, y) in r.t.iter().zip(r.component(0)) {
                assert!((y - (-t).exp()).abs() < 1e-8, "{tspan}: {t} {y}");
            }
            let script = host.app.script(&ws).unwrap();
            assert!(script.contains("[t, y] = ode45(f, "), "{script}");
            assert_eq!(run_script("", &script, "y"), r.component(0));
        }
        // Times must be monotonic.
        let mut host = typed("-y1", &[], "1", "[0 2 1]");
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("increase"), "{error}");
        host.app.spec.tspan = "[3]".into();
        assert!(host.app.script(&ws).unwrap_err().contains("two values"));
    }

    #[test]
    fn solve_ode_app_ode15i_on_an_implicit_problem() {
        let ws = Snapshot::default();
        let mut host = typed("yp1 + 2*y1", &[], "1", "[0 1]");
        host.app.spec.solver = Solver::Ode15i;
        host.app.spec.yp0 = "-2".into();
        solve(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.starts_with(
                "f = @(t, y, yp) yp(1) + 2*y(1);\n[t, y] = ode15i(f, [0 1], 1, -2);\n"
            ),
            "{script}"
        );
        let y = last(&host)[0];
        assert!((y - (-2f64).exp()).abs() < 5e-4, "{y}");
        assert_eq!(run_script("", &script, "y(end)"), [y]);
        let r = solution(&host);
        assert_eq!(r.solver, Solver::Ode15i);
        assert!(r.stats[0] > 0. && r.stats[2] > 0.);
        // The structure output carries the same counts.
        host.app.structure = true;
        solve(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert_eq!(
            run_script(
                "",
                &script,
                "[sol.stats.nsteps sol.stats.nfailed sol.stats.nfevals]"
            ),
            solution(&host).stats
        );
        // yp0 must match y0; events and the y'' helper are not for ode15i.
        host.app.spec.yp0 = "[1 2]".into();
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("yp0 has 2 values")
        );
        host.app.spec.yp0 = "-2".into();
        host.app.spec.events = Events::Condition;
        assert!(!host.app.script(&ws).unwrap().contains("Events"));
    }

    #[test]
    fn solve_ode_app_lists_only_options_that_differ_from_the_defaults() {
        let ws = workspace("opts = 1; M = [2 0; 0 1]; B = eye(3);");
        let mut host = Host::new(SolveOde::default());
        host.app.prepare(&ws);
        let call = |host: &Host<SolveOde>| {
            host.app
                .script(&ws)
                .unwrap()
                .lines()
                .find(|l| l.contains("= ode45("))
                .unwrap()
                .to_owned()
        };
        // Defaults typed out are still defaults: no odeset at all.
        host.app.spec.rel_tol = "1e-3".into();
        host.app.spec.abs_tol = "1e-6".into();
        host.app.prepare(&ws);
        let script = host.app.script(&ws).unwrap();
        assert!(!script.contains("odeset"), "{script}");
        assert_eq!(call(&host), "[t, y] = ode45(f, [0 20], [2 0]);");
        // Changed options, in a fixed order; opts is taken, so opts1.
        host.app.spec.rel_tol = "1e-4".into();
        host.app.spec.abs_tol = "1e-8 1e-9".into();
        host.app.spec.max_step = "0.1".into();
        host.app.spec.initial_step = "0.01".into();
        host.app.prepare(&ws);
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains(
                "opts1 = odeset('RelTol', 0.0001, 'AbsTol', [1e-8 1e-9], 'MaxStep', 0.1, \
                 'InitialStep', 0.01);\n"
            ),
            "{script}"
        );
        assert_eq!(call(&host), "[t, y] = ode45(f, [0 20], [2 0], opts1);");
        solve(&mut host, &ws).unwrap();
        // The mass matrix only reaches ode15s and ode23t.
        host.app.spec.mass = "M".into();
        host.app.prepare(&ws);
        assert!(!host.app.script(&ws).unwrap().contains("Mass"));
        for solver in [Solver::Ode15s, Solver::Ode23t] {
            host.app.spec.solver = solver;
            solve(&mut host, &ws).unwrap_or_else(|e| panic!("{solver:?}: {e}"));
            assert!(host.app.script(&ws).unwrap().contains(", 'Mass', M);"));
            assert_eq!(host.app.inputs(), ["M"]);
        }
        host.app.spec.mass = "B".into();
        host.app.prepare(&ws);
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("B is 3-by-3"), "{error}");
        host.app.spec.mass.clear();
        // NonNegative is written, succeeds on ode23t, and errors on ode23s.
        host.app.spec.non_negative = "1".into();
        host.app.prepare(&ws);
        assert!(host.app.script(&ws).unwrap().contains("'NonNegative', 1"));
        solve(&mut host, &ws).unwrap();
        host.app.spec.solver = Solver::Ode23s;
        let error = solve(&mut host, &ws).unwrap_err();
        assert!(error.contains("NonNegative"), "{error}");
        host.app.spec.non_negative.clear();
        // Bad values are the app's errors.
        #[allow(clippy::type_complexity)]
        let cases: [(fn(&mut Spec), &str); 3] = [
            (
                |s| s.rel_tol = "-1".into(),
                "RelTol must be a positive number",
            ),
            (
                |s| s.abs_tol = "1 2 3".into(),
                "AbsTol needs one value or one per component",
            ),
            (
                |s| s.max_step = "x".into(),
                "MaxStep must be a positive number",
            ),
        ];
        for (set, expected) in cases {
            let mut bad = Host::new(SolveOde::default());
            set(&mut bad.app.spec);
            let error = bad.app.script(&ws).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn solve_ode_app_plot_calls_follow_the_chosen_view() {
        let ws = Snapshot::default();
        let mut host = Host::new(SolveOde::default());
        host.app.plot = Plot::Phase;
        solve(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.ends_with("plot(y(:, 1), y(:, 2))\nxlabel('y1')\nylabel('y2')\n"),
            "{script}"
        );
        run_script("", &script, "y(end, :)");
        host.app.components = [1, 0, 2];
        host.app.prepare(&ws);
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .contains("plot(y(:, 2), y(:, 1))")
        );
        host.app.structure = true;
        host.app.prepare(&ws);
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("sol = ode45(f, [0 20], [2 0]);\nplot(sol.y(2, :), sol.y(1, :))\n"),
            "{script}"
        );
        assert_eq!(host.app.exported(&ws), ["sol", "mu", "f"]);
        run_script("", &script, "sol.y(:, end)");
        // 3-D: the Lorenz system.
        let mut host = typed(
            "s*(y2 - y1)\ny1*(r - y3) - y2\ny1*y2 - b*y3",
            &[("s", "10"), ("r", "28"), ("b", "8/3")],
            "[1 1 1]",
            "[0 2]",
        );
        host.app.plot = Plot::Phase3;
        solve(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.ends_with(
                "plot3(y(:, 1), y(:, 2), y(:, 3))\nxlabel('y1')\nylabel('y2')\nzlabel('y3')\ngrid on\n"
            ),
            "{script}"
        );
        assert_eq!(run_script("", &script, "y(end, :)"), last(&host));
        // Two components cannot make a 3-D plot.
        let mut host = Host::new(SolveOde::default());
        host.app.plot = Plot::Phase3;
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("needs 3 components"), "{error}");
        // Events are marked in the phase plane too.
        host.app.plot = Plot::Phase;
        host.app.spec.events = Events::Condition;
        host.app.spec.event_value = "y2".into();
        host.app.spec.terminal = false;
        solve(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("hold on\nplot(ye(:, 1), ye(:, 2), 'o')\nhold off\n"),
            "{script}"
        );
        assert!(solution(&host).events() > 1);
        assert_eq!(
            run_script("", &script, "numel(te)"),
            [solution(&host).events() as f64]
        );
    }

    #[test]
    fn solve_ode_app_run_stop_and_auto_run() {
        let ws = Snapshot::default();
        let mut host = Host::new(SolveOde::default());
        host.app.prepare(&ws);
        let first = host.request(&ws).unwrap().key;
        // Run solves again: same script, new key.
        host.app.run();
        host.app.prepare(&ws);
        let again = host.request(&ws).unwrap();
        assert_ne!(again.key, first);
        assert!(again.code.contains("% run 1\n"));
        // Stop: no script until Run or an edit.
        host.app.stop();
        host.app.prepare(&ws);
        assert!(host.app.script(&ws).unwrap_err().contains("stopped"));
        host.app.spec.tspan = "[0 10]".into();
        host.app.prepare(&ws);
        assert!(host.app.script(&ws).unwrap().contains("[0 10]"));
        // Auto-run off: edits wait for Run.
        host.app.auto = false;
        host.app.spec.tspan = "[0 5]".into();
        host.app.prepare(&ws);
        assert!(host.app.script(&ws).unwrap().contains("[0 10]"));
        host.app.run();
        host.app.prepare(&ws);
        assert!(host.app.script(&ws).unwrap().contains("[0 5]"));
        // The private run measures the wall time from its prelude.
        assert!(again.code.contains("app_tic = tic;\n"));
    }

    #[test]
    fn solve_ode_app_errors_show_and_keep_the_last_solution() {
        let ws = Snapshot::default();
        let mut host = Host::new(SolveOde::default());
        solve(&mut host, &ws).unwrap();
        let good = last(&host);
        // A typed equation that does not parse never reaches the engine.
        host.app.spec.equations = "y2\n-y1 + * 2".into();
        let error = host.request(&ws).err().unwrap();
        assert!(error.contains("equation 2 does not parse"), "{error}");
        // y0 of the wrong length.
        host.app.spec.equations = "y2\n-y1".into();
        host.app.spec.y0 = "[1 2 3]".into();
        let error = host.app.script(&ws).unwrap_err();
        assert!(
            error.contains("y0 has 3 values but there are 2 equations"),
            "{error}"
        );
        // Parameter names are checked.
        host.app.spec.y0 = "[1 0]".into();
        host.app.spec.parameters = vec![("t".into(), "1".into())];
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("variable of the equations")
        );
        host.app.spec.parameters = vec![("k".into(), String::new())];
        assert!(host.app.script(&ws).unwrap_err().contains("needs a value"));
        host.app.spec.parameters.clear();
        // An engine error: the message reaches the status line; the plot
        // keeps the last good solution.
        host.app.spec.equations = "y2\nundefined_name*y1".into();
        let ctx = egui::Context::default();
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
        assert!(text.contains("undefined_name"), "{text}");
        assert_eq!(last(&host), good);
        // A parse error shows in the status line too.
        host.app.spec.equations = "y2\n-y1 + * 2".into();
        host.refresh(&ctx, &ws);
        let (text, status) = host.status();
        assert_eq!(status, toolkits::AppStatus::Error);
        assert!(text.contains("does not parse"), "{text}");
    }

    fn open_solver(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) -> egui::FullOutput {
        app.toolkits.open_tool(Tool::App(AppKind::SolveOde));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = frame_at(app, ctx, size, vec![]);
            let host = &app.toolkits.apps.ode;
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
    fn solve_ode_app_tab_renders_three_areas() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        let output = open_solver(&mut app, &ctx, size);
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(Tool::App(AppKind::SolveOde))
        );
        for label in [
            "INPUTS",
            "SOLUTION",
            "OUTPUT",
            "Against time",
            "Phase plane",
            "3-D phase",
            "Script",
            "Equations",
            "Parameters",
            "Initial values y0",
            "Solver",
            "Options",
            "Events",
            "Steps",
            "Final state y(tf)",
            "Export to workspace",
            "Generate script",
        ] {
            assert!(painted(&output, label), "{label} not painted");
        }
        let y = apps::short(app.toolkits.apps.ode.app.result.as_ref().unwrap().last()[0]);
        assert!(painted(&output, &y), "y(tf) value {y}");
        let (text, status) = app
            .toolkits
            .app_status(Tool::App(AppKind::SolveOde), false)
            .unwrap();
        assert_eq!(status, toolkits::AppStatus::Success);
        assert!(text.starts_with("ode45 · y(20) = ["), "{text}");
        // Every view renders; the phase views keep the solution.
        for view in ["Phase plane", "3-D phase", "Script", "Against time"] {
            click_label_at(&mut app, &ctx, size, view);
            let output = frame_at(&mut app, &ctx, size, vec![]);
            assert!(painted(&output, view));
        }
        click_label_at(&mut app, &ctx, size, "Script");
        assert!(app.toolkits.apps.ode.app.show_script);
        // A narrow host stacks OUTPUT under the main view and still renders.
        let narrow = egui::vec2(900., 700.);
        let output = frame_at(&mut app, &ctx, narrow, vec![]);
        assert!(painted(&output, "OUTPUT"));
        // Closing the tab keeps the inputs for the next open.
        app.close_tool_tab(Tool::App(AppKind::SolveOde));
        assert!(app.toolkits.tabs().next().is_none());
        assert!(app.toolkits.apps.ode.app.result.is_some());
    }

    #[test]
    fn solve_ode_app_export_does_not_clobber_and_runs_the_event_function() {
        // Five output names make OUTPUT tall: room for its buttons.
        let size = egui::vec2(1440., 1600.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run("t = 7; y = 'keep'; f = 1; te = 2;".into());
        finish(&mut app);
        {
            let spec = &mut app.toolkits.apps.ode.app.spec;
            spec.equations = "y2\n-g".into();
            spec.parameters = vec![("g".into(), "9.81".into())];
            spec.y0 = "[10 0]".into();
            spec.tspan = "[0 5]".into();
            spec.events = Events::Condition;
        }
        open_solver(&mut app, &ctx, size);
        let shown = app.toolkits.apps.ode.app.result.clone().unwrap();
        assert_eq!(shown.events(), 1);
        click_label_at(&mut app, &ctx, size, "Export to workspace");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        assert!(
            matches!(&app.variables["t"], matrust::Array::Numeric(m) if m.data() == [7.]),
            "t is kept: {}",
            app.status
        );
        assert!(matches!(&app.variables["y"], matrust::Array::Character(_)));
        assert!(matches!(&app.variables["f"], matrust::Array::Numeric(m) if m.data() == [1.]));
        let column = |name: &str| match &app.variables[name] {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            other => panic!("{name}: {other:?}"),
        };
        assert_eq!(column("t1"), shown.t);
        assert_eq!(column("y1"), shown.y);
        assert_eq!(column("te1"), shown.te);
        assert_eq!(column("ie"), shown.ie);
        assert!(matches!(&app.variables["f1"], matrust::Array::Handle(_)));
        assert!(app.toolkits.apps.ode.message.contains("t1"));
        // t, y and te are taken now: the next defaults move on.
        frame_at(&mut app, &ctx, size, vec![]);
        let ws = &app.toolkits.apps.workspace;
        let names = app.toolkits.apps.ode.app.output_names(ws).unwrap();
        assert!(names.contains(&(Out::T, "t2".to_owned())), "{names:?}");
        assert!(names.contains(&(Out::Te, "te2".to_owned())), "{names:?}");
    }

    #[test]
    fn solve_ode_app_generate_script_saves_header_and_the_same_script() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("solved.m");
        let mut app = Workspace::with_context(ctx.clone(), true);
        open_solver(&mut app, &ctx, size);
        let script = {
            let ws = &app.toolkits.apps.workspace;
            app.toolkits.apps.ode.app.script(ws).unwrap()
        };
        app.toolkits.apps.test_save_path = Some(path.clone());
        click_label_at(&mut app, &ctx, size, "Generate script");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.starts_with("% Solve ODE (MatRust app), generated "),
            "{saved}"
        );
        assert!(saved.ends_with(&script), "{saved}");
        assert!(
            app.toolkits.apps.ode.message.starts_with("Saved"),
            "{}",
            app.toolkits.apps.ode.message
        );
        // The saved file runs and reproduces y(tf).
        let shown = app.toolkits.apps.ode.app.result.as_ref().unwrap().last();
        assert_eq!(run_script("", &saved, "y(end, :)"), shown);
    }

    #[test]
    fn solve_ode_app_expression_fields_reject_statement_escapes() {
        let ws = Snapshot::default();
        for field in 0..5 {
            let mut host = typed("-k*y1", &[("k", "0.5")], "1", "[0 1]");
            match field {
                0 => host.app.spec.y0 = "1), clear victim, disp(1".into(),
                1 => host.app.spec.tspan = "[0 1]), injected=123; disp(1".into(),
                2 => host.app.spec.parameters[0].1 = "0.5; injected=123".into(),
                3 => {
                    host.app.spec.source = Source::Function;
                    host.app.spec.function = "@(t,y) -y; injected=123".into();
                }
                _ => host.app.spec.rel_tol = "1e-3; injected=123".into(),
            }
            host.app.run();
            host.app.prepare(&ws);
            assert!(host.app.script(&ws).is_err(), "accepted field {field}");
        }
    }

    #[test]
    fn solve_ode_app_rejects_comments_that_swallow_generated_delimiters() {
        let ws = Snapshot::default();
        let mut host = typed("-y1", &[], "1 % comment", "[0 1]");
        host.app.prepare(&ws);
        assert!(host.app.script(&ws).is_err());
    }
}
