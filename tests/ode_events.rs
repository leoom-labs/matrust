//! Closed forms from tool/refs/ode_events/reference.py, cross-checked against SciPy.
use matrust::{Engine, Matrix};

const SOLVERS: [&str; 8] = [
    "ode45", "ode23", "ode113", "ode15s", "ode23s", "ode23t", "ode23tb", "ode15i",
];

fn reference() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/ode_events/reference.json")).unwrap()
}

fn near(got: f64, expected: f64, tolerance: f64) {
    assert!(
        (got - expected).abs() <= tolerance,
        "{got:.17e} != {expected:.17e}"
    );
}

fn run(engine: &mut Engine, source: &str) {
    engine
        .execute(source)
        .unwrap_or_else(|err| panic!("{source}\n{err}"));
}

fn event_folder(helpers: &[(&str, &str, &str)]) -> (tempfile::TempDir, Engine) {
    let folder = tempfile::tempdir().unwrap();
    for &(name, args, body) in helpers {
        std::fs::write(
            folder.path().join(format!("{name}.m")),
            format!("function [v,term,dir]={name}({args})\n{body}\nend\n"),
        )
        .unwrap();
    }
    let mut engine = Engine::default();
    engine.set_current_folder(folder.path()).unwrap();
    (folder, engine)
}

fn shape(matrix: &Matrix) -> (usize, usize) {
    (matrix.rows(), matrix.cols())
}

#[test]
fn ball_stops_at_the_interpolated_ground_crossing() {
    let (_folder, mut engine) = event_folder(&[("ground", "t,y,g", "v=y(1); term=1; dir=-1;")]);
    let expected = reference();
    for solver in SOLVERS {
        let (f, initial) = if solver == "ode15i" {
            ("@(t,y,yp,g) yp-[y(2);-g]", "[10;0],[0;-9.81]")
        } else {
            ("@(t,y,g) [y(2);-g]", "[10;0]")
        };
        run(
            &mut engine,
            &format!(
                "o=odeset('Events',@ground,'RelTol',1e-9,'AbsTol',1e-11); \
             [t,y,te,ye,ie]={solver}({f},[0 0.25 0.75 1.25 1.75 3],{initial},o,9.81);"
            ),
        );
        let vars = &engine.variables;
        assert_eq!(shape(&vars["te"]), (1, 1), "{solver}");
        assert_eq!(shape(&vars["ye"]), (1, 2), "{solver}");
        assert_eq!(vars["ie"].data(), [1.], "{solver}");
        let te = vars["te"].data()[0];
        near(te, expected["impact"].as_f64().unwrap(), 1e-6);
        assert_eq!(vars["t"].data(), [0., 0.25, 0.75, 1.25, te], "{solver}");
        let y = &vars["y"];
        let ye = &vars["ye"];
        assert_eq!(
            [y.get(y.rows() - 1, 0), y.get(y.rows() - 1, 1)],
            ye.data(),
            "{solver}"
        );
        near(ye.data()[0], 0., 1e-11);
        near(
            ye.data()[1],
            expected["impact_velocity"].as_f64().unwrap(),
            1e-5,
        );
    }
}

#[test]
fn bouncing_ball_can_restart_at_the_exact_ground_state() {
    let (_folder, mut engine) = event_folder(&[("ground", "t,y", "v=y(1); term=1; dir=-1;")]);
    run(
        &mut engine,
        "o=odeset('Events',@ground,'RelTol',1e-9,'AbsTol',1e-11,'Refine',5); \
         hit=zeros(3,1); ti=0; yi=[10;0]; \
         for j=1:3; \
           [t,y,te,ye,ie]=ode45(@(t,y) [y(2);-9.81],[ti ti+3],yi,o); \
           hit(j)=te; ti=te; yi=[0;-0.9*ye(2)]; stop=t(end)-te; \
         end;",
    );
    assert_eq!(engine.variables["stop"].data(), [0.]);
    for (&got, expected) in engine.variables["hit"]
        .data()
        .iter()
        .zip(reference()["bounces"].as_array().unwrap())
    {
        near(got, expected.as_f64().unwrap(), 1e-5);
    }
    let t = engine.variables["t"].data();
    assert!(t.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn sine_crossings_ignore_t0_and_respect_direction() {
    let (_folder, mut engine) = event_folder(&[("sine", "t,y,d", "v=sin(t); term=0; dir=d;")]);
    let expected = reference();
    for solver in SOLVERS {
        let (f, initial) = if solver == "ode15i" {
            ("@(t,y,yp,d) yp-1", "0,1")
        } else {
            ("@(t,y,d) 1", "0")
        };
        for direction in [0, 1] {
            run(
                &mut engine,
                &format!(
                    "o=odeset('Events',@sine,'MaxStep',1); \
                 [t,y,te,ye,ie]={solver}({f},[0 10],{initial},o,{direction});"
                ),
            );
            let vars = &engine.variables;
            let indices = if direction == 0 { &[0, 1, 2][..] } else { &[1] };
            assert_eq!(shape(&vars["te"]), (indices.len(), 1), "{solver}");
            assert_eq!(shape(&vars["ie"]), (indices.len(), 1), "{solver}");
            for (i, &j) in indices.iter().enumerate() {
                let time = expected["sine_zeros"][j].as_f64().unwrap();
                near(vars["te"].data()[i], time, 1e-6);
                near(vars["ye"].data()[i], time, 1e-6);
                assert_eq!(vars["ie"].data()[i], 1.);
            }
            assert_eq!(vars["t"].data().last(), Some(&10.), "{solver}");
        }
    }
}

#[test]
fn events_in_one_step_are_sorted_and_stop_at_the_first_terminal_root() {
    let (_folder, mut engine) = event_folder(&[(
        "levels",
        "t,y",
        "v=[t-0.75; t-0.5; t-0.25; 2*t-1]; term=[0;1;0;0]; dir=zeros(4,1);",
    )]);
    for solver in SOLVERS {
        let (f, initial) = if solver == "ode15i" {
            ("@(t,y,yp) yp", "[1;2],[0;0]")
        } else {
            ("@(t,y) 0*y", "[1;2]")
        };
        run(
            &mut engine,
            &format!(
                "o=odeset('Events',@levels,'InitialStep',1,'MaxStep',1,'Refine',3); \
             [t,y,te,ye,ie]={solver}({f},[0 1],{initial},o); \
             s={solver}({f},[0 1],{initial},o); \
             xe=s.xe; sy=s.ye; si=s.ie; steps=s.x; at_stop=deval(s,s.xe(end));"
            ),
        );
        let vars = &engine.variables;
        assert_eq!(vars["te"].data(), [0.25, 0.5, 0.5], "{solver}");
        assert_eq!(vars["ie"].data(), [3., 2., 4.], "{solver}");
        assert_eq!(shape(&vars["ye"]), (3, 2), "{solver}");
        assert_eq!(shape(&vars["xe"]), (1, 3), "{solver}");
        assert_eq!(shape(&vars["sy"]), (2, 3), "{solver}");
        assert_eq!(shape(&vars["si"]), (1, 3), "{solver}");
        assert_eq!(vars["xe"].data(), vars["te"].data());
        for (&value, expected) in vars["sy"].data().iter().zip([1., 2., 1., 2., 1., 2.]) {
            near(value, expected, 5e-13);
        }
        assert_eq!(vars["si"].data(), vars["ie"].data());
        assert_eq!(vars["at_stop"].data(), &vars["sy"].data()[4..]);
        assert_eq!(vars["t"].data(), [0., 0.5 / 3., 0.5 * (2. / 3.), 0.5]);
        assert_eq!(vars["steps"].data(), [0., 0.5], "{solver}");
        let error = engine.execute("deval(s,0.51)").unwrap_err();
        assert!(error.contains("outside the interval"), "{solver}: {error}");
    }
}

#[test]
fn empty_event_outputs_preserve_state_width_with_or_without_options() {
    let (_folder, mut engine) = event_folder(&[("absent", "t,y", "v=1; term=0; dir=0;")]);
    for solver in SOLVERS {
        let (f, initial) = if solver == "ode15i" {
            ("@(t,y,yp) yp", "[1;2],[0;0]")
        } else {
            ("@(t,y) 0*y", "[1;2]")
        };
        for opts in ["[]", "odeset('Events',@absent)"] {
            run(
                &mut engine,
                &format!("[t,y,te,ye,ie]={solver}({f},[0 1],{initial},{opts});"),
            );
            let vars = &engine.variables;
            assert_eq!(shape(&vars["te"]), (0, 1), "{solver}");
            assert_eq!(shape(&vars["ye"]), (0, 2), "{solver}");
            assert_eq!(shape(&vars["ie"]), (0, 1), "{solver}");
        }
    }
}

#[test]
fn extra_parameters_reach_derivative_event_jacobian_and_mass_callbacks() {
    let (_folder, mut engine) = event_folder(&[("level", "t,y,k,m", "v=t-k/m; term=0; dir=1;")]);
    let expected = reference();
    let decay = "@(t,y,k) -k*y";
    run(
        &mut engine,
        &format!("[t,y]=ode45({decay},[0 1],1,[],2); v=y(end);"),
    );
    near(
        engine.variables["v"].data()[0],
        expected["decay"].as_f64().unwrap(),
        1e-5,
    );
    for solver in ["ode15s", "ode23t", "ode23tb"] {
        let mass = if solver == "ode23t" {
            "@(t,y,k,m) m"
        } else {
            "@(t,k,m) m,'MStateDependence','none'"
        };
        run(
            &mut engine,
            &format!(
                "o=odeset('Mass',{mass},'Jacobian',@(t,y,k,m) -k,'Events',@level,'RelTol',1e-8,'AbsTol',1e-10); \
             [t,y,te]={solver}(@(t,y,k,m) -k*y,[0 1],1,o,2,4); v=y(end);"
            ),
        );
        near(
            engine.variables["v"].data()[0],
            expected["mass_decay"].as_f64().unwrap(),
            1e-6,
        );
        assert_eq!(engine.variables["te"].data(), [0.5], "{solver}");
    }
}

#[test]
fn cut_rk_step_keeps_dense_values_and_extra_parameters_through_extension() {
    let (_folder, mut engine) = event_folder(&[("stop_half", "t,y,k", "v=t-0.5; term=1; dir=1;")]);
    for solver in ["ode45", "ode23"] {
        run(
            &mut engine,
            &format!(
                "f=@(t,y,k) k*t; o=odeset('InitialStep',1,'MaxStep',1); \
             s={solver}(f,[0 1],0,odeset(o,'Events',@stop_half),2); \
             [v,d]=deval(s,[0.2 0.5]); \
             s=odextend(s,[],1); w=deval(s,[0.2 0.5 0.9]);"
            ),
        );
        for (&got, want) in engine.variables["v"].data().iter().zip([0.04, 0.25]) {
            near(got, want, 1e-13);
        }
        for (&got, want) in engine.variables["d"].data().iter().zip([0.4, 1.]) {
            near(got, want, 1e-13);
        }
        for (&got, want) in engine.variables["w"].data().iter().zip([0.04, 0.25, 0.81]) {
            near(got, want, 1e-13);
        }
    }
}

#[test]
fn ode15i_events_can_depend_on_the_interpolated_slope() {
    let (_folder, mut engine) = event_folder(&[("speed", "t,y,yp,k", "v=yp-k/2; term=1; dir=1;")]);
    run(
        &mut engine,
        "o=odeset('Events',@speed,'RelTol',1e-9,'AbsTol',1e-11); \
         s=ode15i(@(t,y,yp,k) yp-k*t,[0 1],0,0,o,2); \
         te=s.xe; ye=s.ye; v=deval(s,te); d=s.extdata.ypfinal;",
    );
    near(engine.variables["te"].data()[0], 0.5, 1e-6);
    near(engine.variables["ye"].data()[0], 0.25, 1e-6);
    assert_eq!(engine.variables["ye"].data(), engine.variables["v"].data());
    near(engine.variables["d"].data()[0], 1., 1e-10);
}
