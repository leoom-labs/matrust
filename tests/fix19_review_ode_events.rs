use matrust::Engine;

const ALL_SOLVERS: [&str; 8] = [
    "ode45", "ode23", "ode113", "ode15s", "ode23s", "ode23t", "ode23tb", "ode15i",
];

const LEGACY_SOLVERS: [&str; 5] = ["ode113", "ode15s", "ode23s", "ode23t", "ode23tb"];

fn run(engine: &mut Engine, source: &str) {
    engine
        .execute(source)
        .unwrap_or_else(|err| panic!("{source}\n{err}"));
}

fn near(got: f64, expected: f64, tol: f64, ctx: &str) {
    assert!(
        (got - expected).abs() <= tol,
        "{ctx}: got {got:.17e}, expected {expected:.17e}"
    );
}

fn solver_rhs(solver: &str) -> (&'static str, &'static str) {
    if solver == "ode15i" {
        ("@(t,y,yp) yp - 1", "0, 1")
    } else {
        ("@(t,y) 1", "0")
    }
}

fn temp_engine(files: &[(&str, &str)]) -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().unwrap();
    for &(name, body) in files {
        std::fs::write(dir.path().join(format!("{name}.m")), body).unwrap();
    }
    let mut engine = Engine::default();
    engine.set_current_folder(dir.path()).unwrap();
    (dir, engine)
}

#[test]
fn terminal_event_at_requested_tspan_point_is_not_duplicated() {
    let (_dir, mut engine) = temp_engine(&[(
        "ev_half",
        "function [v,term,dir]=ev_half(t,y)\nv = t - 0.5; term = 1; dir = 0;\nend\n",
    )]);
    for solver in ALL_SOLVERS {
        let (f, init) = solver_rhs(solver);
        run(
            &mut engine,
            &format!(
                "opts = odeset('Events', @ev_half, 'RelTol', 1e-8, 'AbsTol', 1e-10); \
                 [t, y, te, ye, ie] = {solver}({f}, [0 0.5 1], {init}, opts);"
            ),
        );
        let t = engine.variables["t"].data();
        let y = engine.variables["y"].data();
        let te = engine.variables["te"].data();
        let ye = engine.variables["ye"].data();
        let ie = engine.variables["ie"].data();
        assert_eq!(t.len(), 2, "{solver}: expected [0; 0.5], got t={t:?}");
        near(t[0], 0.0, 1e-12, solver);
        near(t[1], 0.5, 1e-10, solver);
        near(y[1], 0.5, 1e-10, solver);
        assert_eq!(te.len(), 1, "{solver}: te={te:?}");
        near(te[0], 0.5, 1e-10, solver);
        near(ye[0], 0.5, 1e-10, solver);
        assert_eq!(ie, vec![1.0], "{solver}");
    }
}

#[test]
fn output_fcn_stop_before_event_truncates_events_to_returned_interval() {
    let (_dir, mut engine) = temp_engine(&[
        (
            "ev_multi",
            "function [v,term,dir]=ev_multi(t,y)\nv = [y - 0.2; y - 0.4]; term = [0; 0]; dir = [0; 0];\nend\n",
        ),
        (
            "ev_after",
            "function [v,term,dir]=ev_after(t,y)\nv = y - 0.4; term = 0; dir = 0;\nend\n",
        ),
        (
            "stop_at_025",
            "function status=stop_at_025(t,y,flag)\nif isempty(flag)\n  status = any(t >= 0.25 - 1e-12);\nelse\n  status = 0;\nend\nend\n",
        ),
    ]);
    for solver in ALL_SOLVERS {
        let (f, init) = solver_rhs(solver);
        // One 0.5 step covers the stop at 0.25 and the event at 0.4 (default MaxStep would stop first).
        run(
            &mut engine,
            &format!(
                "opts = odeset('Events', @ev_after, 'OutputFcn', @stop_at_025, 'InitialStep', 0.5, 'MaxStep', 0.5); \
                 [t, y, te, ye, ie] = {solver}({f}, [0 0.25 0.9 1], {init}, opts);"
            ),
        );
        let t = engine.variables["t"].data();
        let te = engine.variables["te"].data();
        let ye = engine.variables["ye"].data();
        let ie = engine.variables["ie"].data();
        assert_eq!(t.len(), 2, "{solver}: t={t:?}");
        near(t[1], 0.25, 1e-12, solver);
        assert!(te.is_empty(), "{solver}: expected empty te, got {te:?}");
        assert!(ye.is_empty(), "{solver}: expected empty ye, got {ye:?}");
        assert!(ie.is_empty(), "{solver}: expected empty ie, got {ie:?}");

        // Event at 0.2 is inside [0, 0.25], event at 0.4 is outside -> only 0.2 is retained.
        run(
            &mut engine,
            &format!(
                "opts = odeset('Events', @ev_multi, 'OutputFcn', @stop_at_025, 'InitialStep', 0.5, 'MaxStep', 0.5); \
                 [t, y, te, ye, ie] = {solver}({f}, [0 0.25 0.9 1], {init}, opts);"
            ),
        );
        let t = engine.variables["t"].data();
        let te = engine.variables["te"].data();
        let ie = engine.variables["ie"].data();
        assert_eq!(t.len(), 2, "{solver}: t={t:?}");
        assert_eq!(
            te.len(),
            1,
            "{solver}: expected only t=0.2 event, got te={te:?}"
        );
        near(te[0], 0.2, 1e-9, solver);
        assert_eq!(ie, vec![1.0], "{solver}");
    }
}

#[test]
fn nonterminal_event_at_root_with_terminal_at_step_end_does_not_stop() {
    let (_dir, mut engine) = temp_engine(&[(
        "ev_state_term",
        "function [v,term,dir]=ev_state_term(t,y)\nv = y - 0.75; term = double(t >= 0.9); dir = 0;\nend\n",
    )]);
    for solver in ALL_SOLVERS {
        let (f, init) = solver_rhs(solver);
        run(
            &mut engine,
            &format!(
                "opts = odeset('Events', @ev_state_term, 'InitialStep', 1, 'MaxStep', 1); \
                 [t, y, te, ye, ie] = {solver}({f}, [0 1], {init}, opts);"
            ),
        );
        let t = engine.variables["t"].data();
        let te = engine.variables["te"].data();
        let ye = engine.variables["ye"].data();
        let ie = engine.variables["ie"].data();
        near(*t.last().unwrap(), 1.0, 1e-12, solver);
        assert_eq!(te.len(), 1, "{solver}: te={te:?}");
        near(te[0], 0.75, 1e-9, solver);
        near(ye[0], 0.75, 1e-9, solver);
        assert_eq!(ie, vec![1.0], "{solver}");
    }
}

#[test]
fn legacy_solvers_request_three_outputs_from_events_at_setup() {
    let (_dir, mut engine) = temp_engine(&[(
        "ev_three_only",
        "function [v,term,dir]=ev_three_only(t,y)\nif nargout ~= 3\n  error('expected 3 outputs');\nend\nv = y - 0.5; term = 1; dir = 0;\nend\n",
    )]);
    for solver in LEGACY_SOLVERS {
        let (f, init) = solver_rhs(solver);
        run(
            &mut engine,
            &format!(
                "opts = odeset('Events', @ev_three_only); \
                 [t, y, te, ye, ie] = {solver}({f}, [0 1], {init}, opts);"
            ),
        );
        let te = engine.variables["te"].data();
        let ye = engine.variables["ye"].data();
        let ie = engine.variables["ie"].data();
        assert_eq!(te.len(), 1, "{solver}: te={te:?}");
        near(te[0], 0.5, 1e-9, solver);
        near(ye[0], 0.5, 1e-9, solver);
        assert_eq!(ie, vec![1.0], "{solver}");
    }
}

#[test]
fn sol_struct_with_tspan_points_passes_requested_points_to_output_fcn() {
    let (_dir, mut engine) = temp_engine(&[(
        "record_out",
        "function status=record_out(t,y,flag)\nglobal rec_t;\nif strcmp(flag, 'init')\n  rec_t = [];\nelseif isempty(flag)\n  rec_t = [rec_t, t];\nend\nstatus = 0;\nend\n",
    )]);
    for solver in ALL_SOLVERS {
        let (f, init) = solver_rhs(solver);
        run(
            &mut engine,
            &format!(
                "global rec_t; rec_t = []; \
                 opts = odeset('OutputFcn', @record_out); \
                 sol = {solver}({f}, [0 0.35 1], {init}, opts); \
                 sol_x = sol.x;"
            ),
        );
        let rec_t = engine.variables["rec_t"].data();
        let sol_x = engine.variables["sol_x"].data();
        assert_eq!(rec_t.len(), 2, "{solver}: rec_t={rec_t:?}");
        near(rec_t[0], 0.35, 1e-12, solver);
        near(rec_t[1], 1.0, 1e-12, solver);
        near(sol_x[0], 0.0, 1e-12, solver);
        near(*sol_x.last().unwrap(), 1.0, 1e-12, solver);
    }
}

#[test]
fn numeric_nonzero_output_fcn_status_stops_integration() {
    let (_dir, mut engine) = temp_engine(&[
        (
            "stop_uint8",
            "function status=stop_uint8(t,y,flag)\nif isempty(flag)\n  status = uint8(1);\nelse\n  status = uint8(0);\nend\nend\n",
        ),
        (
            "stop_double",
            "function status=stop_double(t,y,flag)\nif isempty(flag)\n  status = 1;\nelse\n  status = 0;\nend\nend\n",
        ),
        (
            "cont_uint8",
            "function status=cont_uint8(t,y,flag)\nstatus = uint8(0);\nend\n",
        ),
    ]);
    for solver in ALL_SOLVERS {
        let (f, init) = solver_rhs(solver);
        for fcn in ["stop_uint8", "stop_double"] {
            run(
                &mut engine,
                &format!(
                    "opts = odeset('OutputFcn', @{fcn}); \
                     [t, y] = {solver}({f}, [0 0.35 1], {init}, opts);"
                ),
            );
            let t = engine.variables["t"].data();
            assert_eq!(
                t.len(),
                2,
                "{solver} {fcn}: expected stop at 0.35, got t={t:?}"
            );
            near(t[1], 0.35, 1e-12, solver);
        }
        run(
            &mut engine,
            &format!(
                "opts = odeset('OutputFcn', @cont_uint8); \
                 [t, y] = {solver}({f}, [0 0.35 1], {init}, opts);"
            ),
        );
        let t = engine.variables["t"].data();
        assert_eq!(t.len(), 3, "{solver} cont_uint8: t={t:?}");
        near(t[2], 1.0, 1e-12, solver);
    }
}
