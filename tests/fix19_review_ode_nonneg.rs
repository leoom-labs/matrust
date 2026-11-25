use matrust::Engine;

const NONNEG_SOLVERS: [&str; 6] = ["ode45", "ode23", "ode113", "ode15s", "ode23t", "ode23tb"];

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
fn nonnegative_backward_tspan_respects_integration_direction() {
    let mut engine = Engine::default();
    // Backward integration [1 0] from y(1)=0 with dy/dt = -1: y(t) = 1 - t >= 0,
    // so slope dy/dt = -1 moves into the nonnegative region and must NOT be clamped.
    for solver in NONNEG_SOLVERS {
        run(
            &mut engine,
            &format!(
                "opts = odeset('NonNegative', 1); \
                 [t, y] = {solver}(@(t, y) -1, [1 0], 0, opts);"
            ),
        );
        let y_end = *engine.variables["y"].data().last().unwrap();
        near(y_end, 1.0, 1e-4);

        // Conversely, dy/dt = +1 on [1 0] from y(1)=0 would go negative as t decreases,
        // so it must be clamped to 0.
        run(
            &mut engine,
            &format!(
                "opts = odeset('NonNegative', 1); \
                 [t2, y2] = {solver}(@(t, y) 1, [1 0], 0, opts);"
            ),
        );
        for &yi in engine.variables["y2"].data() {
            assert!(yi >= 0., "{solver}: negative y={yi} in backward clamp");
            near(yi, 0.0, 1e-12);
        }
    }
}

#[test]
fn deval_applies_recorded_nonnegative_constraint() {
    let mut engine = Engine::default();
    for solver in NONNEG_SOLVERS {
        run(
            &mut engine,
            &format!(
                "opts = odeset('NonNegative', 1); \
                 sol = {solver}(@(t, y) -abs(y), [0 40], 1, opts); \
                 tq = linspace(sol.x(1), sol.x(end), 400); \
                 [yq, ypq] = deval(sol, tq); \
                 yq_sub = deval(sol, tq, 1);"
            ),
        );
        for &v in engine.variables["yq"].data() {
            assert!(v >= 0., "{solver}: deval returned negative value {v}");
        }
        for &v in engine.variables["yq_sub"].data() {
            assert!(
                v >= 0.,
                "{solver}: deval(..., idx) returned negative value {v}"
            );
        }
    }
}

#[test]
fn odextend_preserves_nonnegative_for_all_six_solvers() {
    let mut engine = Engine::default();
    for solver in NONNEG_SOLVERS {
        run(
            &mut engine,
            &format!(
                "opts = odeset('NonNegative', 1, 'RelTol', 1e-6, 'AbsTol', 1e-8); \
                 sol = {solver}(@(t, y) -abs(y), [0 2], 1, opts); \
                 sol2 = odextend(sol, [], 5); \
                 t_end = sol2.x(end); \
                 y_end = sol2.y(end); \
                 idx = sol2.idata.idxNonNegative; \
                 y_mid = deval(sol2, 3.5);"
            ),
        );
        near(engine.variables["t_end"].data()[0], 5.0, 1e-12);
        assert!(engine.variables["y_end"].data()[0] >= 0., "{solver}");
        assert_eq!(engine.variables["idx"].data(), [1.], "{solver}");
        let y_mid = engine.variables["y_mid"].data()[0];
        assert!(y_mid >= 0., "{solver}: negative y_mid={y_mid}");
        near(y_mid, (-3.5_f64).exp(), 1e-4);
    }
}

#[test]
fn ode113_supports_mass_with_nonnegative() {
    let mut engine = Engine::default();
    run(
        &mut engine,
        "opts = odeset('Mass', 2, 'NonNegative', 1, 'RelTol', 1e-6, 'AbsTol', 1e-8); \
         [t, y] = ode113(@(t, y) -2 * y, [0 1], 1, opts);",
    );
    near(
        *engine.variables["y"].data().last().unwrap(),
        (-1.0_f64).exp(),
        1e-4,
    );

    run(
        &mut engine,
        "M = [2 0; 0 3]; \
         opts = odeset('Mass', M, 'NonNegative', [1 2], 'RelTol', 1e-6, 'AbsTol', 1e-8); \
         [t2, y2] = ode113(@(t, y) [-2 * y(1); -3 * abs(y(2))], [0 2], [1; 1], opts);",
    );
    let y2 = &engine.variables["y2"];
    let rows = y2.rows();
    for &v in y2.data() {
        assert!(v >= 0., "ode113 Mass+NonNegative produced negative {v}");
    }
    near(y2.data()[rows - 1], (-2.0_f64).exp(), 1e-4);
    near(y2.data()[2 * rows - 1], (-2.0_f64).exp(), 1e-4);

    // ode15s, ode23t, ode23tb still reject NonNegative with Mass.
    for solver in ["ode15s", "ode23t", "ode23tb"] {
        let err = engine
            .execute(&format!(
                "{solver}(@(t, y) -y, [0 1], 1, odeset('Mass', 1, 'NonNegative', 1));"
            ))
            .unwrap_err();
        assert!(err.contains("NonNegative"), "{solver}: {err}");
    }
}

#[test]
fn nonnegative_and_outputsel_reject_matrices() {
    let mut engine = Engine::default();
    for solver in ["ode45", "ode15s"] {
        let err = engine
            .execute(&format!(
                "{solver}(@(t, y) -y, [0 1], [1; 1], odeset('NonNegative', [1 2; 1 2]));"
            ))
            .unwrap_err();
        assert!(
            err.contains("NonNegative must be a vector"),
            "{solver}: {err}"
        );

        let err = engine
            .execute(&format!(
                "{solver}(@(t, y) -y, [0 1], [1; 1], odeset('OutputSel', [1 2; 1 2]));"
            ))
            .unwrap_err();
        assert!(
            err.contains("OutputSel must be a vector"),
            "{solver}: {err}"
        );
    }

    // Scalar, row vector, and column vector are accepted.
    run(
        &mut engine,
        "ode45(@(t, y) -y, [0 0.5], [1; 1], odeset('NonNegative', [1; 2])); \
         ode15s(@(t, y) -y, [0 0.5], [1; 1], odeset('NonNegative', [1 2]));",
    );
}

#[test]
fn output_fcn_checks_element_limit_before_allocating() {
    let (_dir, mut engine) = folder_with(&[(
        "noop_out",
        "function status = noop_out(t, y, flag)\n\
         status = 0;\n\
         end\n",
    )]);
    // OutputSel with 100000 indices and Refine 1000 gives 100M elements per step > MAX_ELEMENTS.
    let err = engine
        .execute(
            "opts = odeset('OutputFcn', @noop_out, 'OutputSel', ones(1, 100000), 'Refine', 1000); \
             [t, y] = ode15s(@(t, y) -y, [0 1], 1, opts);",
        )
        .unwrap_err();
    assert!(err.contains("element limit"), "{err}");
}
