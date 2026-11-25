use matrust::{Engine, Matrix};

const NONNEG_SOLVERS: [&str; 6] = ["ode45", "ode23", "ode113", "ode15s", "ode23t", "ode23tb"];
const ALL_SOLVERS: [&str; 8] = [
    "ode45", "ode23", "ode113", "ode15s", "ode23s", "ode23t", "ode23tb", "ode15i",
];

fn run(engine: &mut Engine, source: &str) {
    engine
        .execute(source)
        .unwrap_or_else(|err| panic!("{source}\n{err}"));
}

fn near(got: f64, expected: f64, tolerance: f64) {
    assert!(
        (got - expected).abs() <= tolerance,
        "{got:.17e} != {expected:.17e} (diff={:.3e})",
        (got - expected).abs()
    );
}

fn folder_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().unwrap();
    for &(name, body) in files {
        std::fs::write(dir.path().join(format!("{name}.m")), body).unwrap();
    }
    let mut engine = Engine::default();
    engine.set_current_folder(dir.path()).unwrap();
    (dir, engine)
}

#[test]
fn nonnegative_abs_decay_stays_nonnegative_and_matches_exp() {
    let mut engine = Engine::default();
    // Without NonNegative, ode45 on dy = -abs(y), [0 40], y0 = 1 undershoots below 0 and diverges.
    run(
        &mut engine,
        "opts0 = odeset('Refine', 1); \
         [t0, y0] = ode45(@(t, y) -abs(y), [0 40], 1, opts0);",
    );
    let min_without = engine.variables["y0"]
        .data()
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    assert!(
        min_without < 0.,
        "expected undershoot without NonNegative, got min={min_without}"
    );

    // With NonNegative, all supported solvers stay >= 0 and match exp(-t) within 1e-4.
    for solver in NONNEG_SOLVERS {
        run(
            &mut engine,
            &format!(
                "opts = odeset('NonNegative', 1, 'RelTol', 1e-6, 'AbsTol', 1e-8); \
                 [t, y] = {solver}(@(t, y) -abs(y), [0 40], 1, opts);"
            ),
        );
        let t = engine.variables["t"].data();
        let y = engine.variables["y"].data();
        assert_eq!(t.len(), y.len(), "{solver}");
        for (&ti, &yi) in t.iter().zip(y.iter()) {
            assert!(yi >= 0., "{solver}: negative y={yi} at t={ti}");
            near(yi, (-ti).exp(), 1e-4);
        }
        // Check sol.idata.idxNonNegative is recorded.
        run(
            &mut engine,
            &format!(
                "sol = {solver}(@(t, y) -abs(y), [0 5], 1, opts); idx = sol.idata.idxNonNegative;"
            ),
        );
        assert_eq!(engine.variables["idx"].data(), [1.], "{solver}");
    }

    run(
        &mut engine,
        "[t_done, y_done] = ode45(@(t, y) -y, [0 1], 1, odeset('NonNegative', 1));",
    );
    near(
        *engine.variables["y_done"].data().last().unwrap(),
        (-1.0_f64).exp(),
        1e-4,
    );
}

#[test]
fn nonnegative_unsupported_combos_error() {
    let mut engine = Engine::default();
    // ode23s and ode15i do not support NonNegative.
    let err = engine
        .execute("ode23s(@(t, y) -y, [0 1], 1, odeset('NonNegative', 1));")
        .unwrap_err();
    assert!(err.contains("NonNegative"), "{err}");

    let err = engine
        .execute("ode15i(@(t, y, yp) yp + y, [0 1], 1, -1, odeset('NonNegative', 1));")
        .unwrap_err();
    assert!(err.contains("NonNegative"), "{err}");

    // ode15s, ode23t, ode23tb do not support NonNegative with a Mass matrix.
    for solver in ["ode15s", "ode23t", "ode23tb"] {
        let err = engine
            .execute(&format!(
                "{solver}(@(t, y) -y, [0 1], 1, odeset('NonNegative', 1, 'Mass', 1));"
            ))
            .unwrap_err();
        assert!(err.contains("NonNegative"), "{solver}: {err}");
    }

    // Initial condition < 0 violates NonNegative at t0.
    let err = engine
        .execute("ode45(@(t, y) -y, [0 1], -0.1, odeset('NonNegative', 1));")
        .unwrap_err();
    assert!(
        err.contains("t0") || err.contains("Non-negativity"),
        "{err}"
    );
}

#[test]
fn output_fcn_records_init_step_done_and_matches_output_rows() {
    let (_dir, mut engine) = folder_with(&[(
        "rec_out",
        "function status = rec_out(t, y, flag, scale)\n\
         global LOG_FLAGS LOG_T LOG_Y LOG_SCALE;\n\
         if strcmp(flag, 'init')\n\
           LOG_FLAGS = {'init'};\n\
           LOG_T = t(1);\n\
           LOG_Y = y.';\n\
           LOG_SCALE = scale;\n\
         elseif isempty(flag)\n\
           LOG_FLAGS{end+1} = 'step';\n\
           LOG_T = [LOG_T; t(:)];\n\
           LOG_Y = [LOG_Y; y.'];\n\
         elseif strcmp(flag, 'done')\n\
           LOG_FLAGS{end+1} = 'done';\n\
         end\n\
         status = 0;\n\
         end\n",
    )]);

    for solver in ALL_SOLVERS {
        let (f, initial) = if solver == "ode15i" {
            ("@(t, y, yp, scale) yp + scale * y", "1, -2")
        } else {
            ("@(t, y, scale) -scale * y", "1")
        };
        run(
            &mut engine,
            &format!(
                "global LOG_FLAGS LOG_T LOG_Y LOG_SCALE;\n\
                 LOG_FLAGS = {{}}; LOG_T = []; LOG_Y = []; LOG_SCALE = 0;\n\
                 opts = odeset('OutputFcn', @rec_out, 'Refine', 2);\n\
                 [t, y] = {solver}({f}, [0 1], {initial}, opts, 2);\n\
                 n_flags = numel(LOG_FLAGS);\n\
                 first_flag = LOG_FLAGS{{1}};\n\
                 last_flag = LOG_FLAGS{{end}};\n\
                 diff_t = max(abs(LOG_T - t));\n\
                 diff_y = max(abs(LOG_Y(:) - y(:)));"
            ),
        );
        assert!(engine.variables["n_flags"].data()[0] >= 3., "{solver}");
        assert_eq!(engine.variables["diff_t"].data()[0], 0., "{solver}");
        assert_eq!(engine.variables["diff_y"].data()[0], 0., "{solver}");
        assert_eq!(engine.variables["LOG_SCALE"].data()[0], 2., "{solver}");
    }
}

#[test]
fn output_fcn_stop_on_true_and_terminal_event_calls_done() {
    let (_dir, mut engine) = folder_with(&[
        (
            "stop_at_half",
            "function status = stop_at_half(t, y, flag)\n\
             global DONE_CALLED LAST_T;\n\
             status = 0;\n\
             if strcmp(flag, 'init')\n\
               DONE_CALLED = 0;\n\
               LAST_T = [];\n\
             elseif isempty(flag)\n\
               LAST_T = [LAST_T; t(:)];\n\
               if any(t >= 0.5)\n\
                 status = 1;\n\
               end\n\
             elseif strcmp(flag, 'done')\n\
               DONE_CALLED = DONE_CALLED + 1;\n\
             end\n\
             end\n",
        ),
        (
            "ev_term",
            "function [v, isterm, dir] = ev_term(t, y)\n\
             v = y - 0.5; isterm = 1; dir = -1;\n\
             end\n",
        ),
    ]);

    run(
        &mut engine,
        "global DONE_CALLED LAST_T;\n\
         opts = odeset('OutputFcn', @stop_at_half, 'MaxStep', 0.1);\n\
         [t, y] = ode45(@(t, y) -y, [0 2], 1, opts);\n\
         diff_last = max(abs(LAST_T - t(2:end)));",
    );
    let t = engine.variables["t"].data();
    assert!(*t.last().unwrap() >= 0.5);
    assert!(*t.last().unwrap() < 2.0);
    assert_eq!(engine.variables["DONE_CALLED"].data(), [1.]);
    assert_eq!(engine.variables["diff_last"].data(), [0.]);

    // Terminal event also calls 'done'.
    run(
        &mut engine,
        "global DONE_CALLED LAST_T;\n\
         opts = odeset('OutputFcn', @stop_at_half, 'Events', @ev_term);\n\
         [t, y, te, ye, ie] = ode45(@(t, y) -1, [0 2], 1, opts);",
    );
    assert_eq!(engine.variables["DONE_CALLED"].data(), [1.]);
    near(*engine.variables["te"].data().last().unwrap(), 0.5, 1e-12);
}

#[test]
fn output_sel_restricts_columns() {
    let (_dir, mut engine) = folder_with(&[(
        "rec_sel",
        "function status = rec_sel(t, y, flag)\n\
         global SEL_Y;\n\
         if strcmp(flag, 'init')\n\
           SEL_Y = y.';\n\
         elseif isempty(flag)\n\
           SEL_Y = [SEL_Y; y.'];\n\
         end\n\
         status = 0;\n\
         end\n",
    )]);
    run(
        &mut engine,
        "global SEL_Y;\n\
         opts = odeset('OutputFcn', @rec_sel, 'OutputSel', [3 1]);\n\
         [t, y] = ode45(@(t, y) [-y(1); 0.5; -2*y(3)], [0 1], [1; 2; 3], opts);\n\
         diff_sel = max(abs(SEL_Y(:) - [y(:,3); y(:,1)]));",
    );
    assert_eq!(engine.variables["SEL_Y"].cols(), 2);
    assert_eq!(engine.variables["diff_sel"].data(), [0.]);
}

#[test]
fn odeprint_prints_t_and_y_for_two_step_case() {
    let mut engine = Engine::default();
    let exec = engine
        .execute(
            "opts = odeset('OutputFcn', @odeprint, 'Refine', 1);\n\
             [t, y] = ode45(@(t, y) -y, [0 0.5 1], 1, opts);",
        )
        .unwrap();
    let printed = exec.output.join("\n");
    let t_count = printed.matches("t =").count();
    let y_count = printed.matches("y =").count();
    assert!(t_count >= 2, "expected t = blocks, got:\n{printed}");
    assert_eq!(t_count, y_count, "mismatched t/y blocks:\n{printed}");
}

#[test]
fn odeplot_leaves_line_with_xdata_and_ydata_equal_to_t_and_y() {
    let mut engine = Engine::default();
    run(
        &mut engine,
        "figure;\n\
         opts = odeset('OutputFcn', @odeplot);\n\
         [t, y] = ode45(@(t, y) -y, [0 1], 1, opts);\n\
         ax = gca;\n\
         h = get(ax, 'Children');\n\
         xd = get(h(1), 'XData');\n\
         yd = get(h(1), 'YData');\n\
         diff_x = max(abs(xd(:) - t(:)));\n\
         diff_y = max(abs(yd(:) - y(:)));",
    );
    let xd: &Matrix = &engine.variables["xd"];
    let t: &Matrix = &engine.variables["t"];
    assert_eq!(xd.len(), t.len());
    assert_eq!(engine.variables["diff_x"].data(), [0.]);
    assert_eq!(engine.variables["diff_y"].data(), [0.]);

    // Also test 'odeplot' as text name with OutputSel on a 2-component system.
    run(
        &mut engine,
        "figure;\n\
         opts = odeset('OutputFcn', 'odeplot', 'OutputSel', 2);\n\
         [t2, y2] = ode23(@(t, y) [-y(1); -2*y(2)], [0 1], [1; 3], opts);\n\
         ax2 = gca;\n\
         h2 = get(ax2, 'Children');\n\
         xd2 = get(h2(1), 'XData');\n\
         yd2 = get(h2(1), 'YData');\n\
         diff_x2 = max(abs(xd2(:) - t2(:)));\n\
         diff_y2 = max(abs(yd2(:) - y2(:, 2)));",
    );
    assert_eq!(engine.variables["diff_x2"].data(), [0.]);
    assert_eq!(engine.variables["diff_y2"].data(), [0.]);
}
