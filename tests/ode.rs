//! ode45/ode23/odeset/odeget. Exact-solution and structural checks, plus
//! reference values in tests/fixtures/ode/. The comparison test replays
//! call1; the error and semantics tests replay call2.
use matrust::{Array, Engine, Matrix};

const ODESET_FIELDS: [&str; 22] = [
    "AbsTol",
    "BDF",
    "Events",
    "InitialStep",
    "Jacobian",
    "JConstant",
    "JPattern",
    "Mass",
    "MassSingular",
    "MaxOrder",
    "MaxStep",
    "NonNegative",
    "NormControl",
    "OutputFcn",
    "OutputSel",
    "Refine",
    "RelTol",
    "Stats",
    "Vectorized",
    "MStateDependence",
    "MvPattern",
    "InitialSlope",
];

const FUNCTIONS: [(&str, &str); 6] = [
    (
        "vdp",
        "function dy=vdp(t,y)\ndy=[y(2); (1-y(1)^2)*y(2)-y(1)];\nend\n",
    ),
    ("expdecay", "function dy=expdecay(t,y)\ndy=-y;\nend\n"),
    (
        "lin2",
        "function dy=lin2(t,y)\ndy=[-0.5*y(1)+2*y(2); -2*y(1)-0.5*y(2)];\nend\n",
    ),
    ("rowout", "function dy=rowout(t,y)\ndy=y.';\nend\n"),
    ("longout", "function dy=longout(t,y)\ndy=[y; 1];\nend\n"),
    ("blowup", "function dy=blowup(t,y)\ndy=y^2;\nend\n"),
];

fn engine() -> (tempfile::TempDir, Engine) {
    let folder = tempfile::tempdir().unwrap();
    for (name, source) in FUNCTIONS {
        std::fs::write(folder.path().join(format!("{name}.m")), source).unwrap();
    }
    let mut e = Engine::default();
    e.set_function_directory(folder.path()).unwrap();
    (folder, e)
}
fn get(e: &Engine, name: &str) -> Matrix {
    e.variables[name].clone()
}
fn error(e: &mut Engine, source: &str) -> String {
    let message = e.execute(source).unwrap_err();
    // Drop the "Line L, column C: " location the session prefixes.
    match message
        .strip_prefix("Line ")
        .and_then(|m| m.split_once(": "))
    {
        Some((_, text)) => text.to_string(),
        None => message,
    }
}

#[test]
fn exp_decay_follows_exact_solution_on_refined_output() {
    let (_folder, mut e) = engine();
    e.execute("[t,y]=ode45('expdecay',[0 5],1);[t2,y2]=ode23('expdecay',[0 5],1);")
        .unwrap();
    for (t, y, bound) in [("t", "y", 2e-4), ("t2", "y2", 2e-3)] {
        let (t, y) = (get(&e, t), get(&e, y));
        assert_eq!((t.cols(), y.cols(), y.rows()), (1, 1, t.rows()));
        assert_eq!((t.data()[0], *t.data().last().unwrap()), (0., 5.));
        assert!(t.data().windows(2).all(|w| w[1] > w[0]));
        let worst = t
            .data()
            .iter()
            .zip(y.data())
            .map(|(t, y)| (y - (-t).exp()).abs())
            .fold(0., f64::max);
        assert!(worst < bound, "{worst}");
    }
    // ode45 refines each step into 4 points; ode23 outputs its steps only.
    assert_eq!(get(&e, "t").rows() % 4, 1);
}

#[test]
fn refine_interpolates_inside_the_same_steps() {
    let (_folder, mut e) = engine();
    e.execute(
        "[t4,y4]=ode45('vdp',[0 20],[2;0]);[t1,y1]=ode45('vdp',[0 20],[2;0],odeset('Refine',1));\
         [t2,y2]=ode45('vdp',[0 20],[2;0],odeset('Refine',2));",
    )
    .unwrap();
    let (t4, y4, t1, y1, t2) = (
        get(&e, "t4"),
        get(&e, "y4"),
        get(&e, "t1"),
        get(&e, "y1"),
        get(&e, "t2"),
    );
    let steps = t1.rows() - 1;
    assert_eq!(
        (t4.rows(), t2.rows(), y4.cols()),
        (4 * steps + 1, 2 * steps + 1, 2)
    );
    for i in 0..=steps {
        assert_eq!(t4.data()[4 * i], t1.data()[i]);
        assert_eq!(t2.data()[2 * i], t1.data()[i]);
        for c in 0..2 {
            assert_eq!(y4.get(4 * i, c), y1.get(i, c));
        }
    }
    // The interpolant is continuous with the step end points.
    let n = t4.rows();
    assert!((y4.get(n - 2, 0) - y4.get(n - 1, 0)).abs() < 0.2);
}

#[test]
fn linear_system_meets_tight_tolerances_and_requested_times() {
    let (_folder, mut e) = engine();
    e.execute(
        "o=odeset('RelTol',1e-10,'AbsTol',1e-12);ts=0:0.5:10;\
         [t,y]=ode45('lin2',ts,[1;0],o);[t3,y3]=ode23('lin2',ts,[1 0],o);",
    )
    .unwrap();
    for (t, y, bound) in [("t", "y", 1e-8), ("t3", "y3", 1e-7)] {
        let (t, y) = (get(&e, t), get(&e, y));
        assert_eq!((t.rows(), t.cols(), y.rows(), y.cols()), (21, 1, 21, 2));
        for i in 0..21 {
            let s = 0.5 * i as f64;
            assert_eq!(t.data()[i], s);
            let decay = (-0.5 * s).exp();
            assert!((y.get(i, 0) - decay * (2. * s).cos()).abs() < bound);
            assert!((y.get(i, 1) + decay * (2. * s).sin()).abs() < bound);
        }
    }
}

#[test]
fn backward_tspan_scalar_tspan_and_single_output() {
    let (_folder, mut e) = engine();
    e.execute(
        "[t,y]=ode45('expdecay',[1 0],1);s=ode45('expdecay',[0 2],1);[u,v]=ode45('expdecay',2,1);",
    )
    .unwrap();
    let (t, y) = (get(&e, "t"), get(&e, "y"));
    assert_eq!((t.data()[0], *t.data().last().unwrap()), (1., 0.));
    assert!(t.data().windows(2).all(|w| w[1] < w[0]));
    assert!((y.data().last().unwrap() - 1f64.exp()).abs() < 1e-3);
    // One output with a function name is t, as MATLAB's ODE-file form.
    let s = get(&e, "s");
    assert_eq!(s.cols(), 1);
    // A scalar tspan integrates from 0 to it.
    assert_eq!(get(&e, "u").data(), s.data());
    assert_eq!(get(&e, "v").rows(), s.rows());
}

#[test]
fn local_functions_and_row_or_matrix_initial_values() {
    let mut e = Engine::default();
    e.execute(
        "[t,y]=ode45('f',[0 1],[1 2 3 4]);[t2,y2]=ode45('f',[0 1],[1 3;2 4]);\
         function d=f(t,y)\nd=-y;\nend",
    )
    .unwrap();
    let (y, y2) = (get(&e, "y"), get(&e, "y2"));
    assert_eq!((y.cols(), y2.cols()), (4, 4));
    assert_eq!(y.data(), y2.data());
    assert_eq!((y.get(0, 0), y.get(0, 3)), (1., 4.));
}

#[test]
fn options_set_merge_and_get() {
    let (_folder, mut e) = engine();
    e.execute(
        "o=odeset('reltol',1e-6,'AbsTol',[1e-8 1e-9]);a=odeget(o,'RelTol');b=odeget(o,'abs');\
         c=odeget(o,'MaxStep');d=odeget(o,'MaxStep',0.25);p=odeset(o,'RelTol',1e-4,'Refine',2);\
         q=odeget(p,'RelTol');r=odeget(p,'Refine');w=odeset(o,odeset('RelTol',1e-2));x=odeget(w,'RelTol');\
         z=odeset(o,'RelTol',[]);g=odeget(z,'RelTol',7);h=odeget([],'Refine',3);k=odeget(o,'Events',5);\
         dot=o.RelTol;kept=p.AbsTol;m=odeget(odeset(o,[]),'RelTol');z0=odeset();n=numfields(z0);\
         em=1*isempty(z0.RelTol);cls=1*isstruct(o);v=odeget([],'NoSuchOption');\
         [ta,ya]=ode45('expdecay',[0 5],1,struct('RelTol',1e-6));\
         [tb,yb]=ode45('expdecay',[0 5],1,odeset('RelTol',1e-6));\
         [tc,yc]=ode45('expdecay',[0 5],1);",
    )
    .unwrap();
    let scalar = |n: &str| get(&e, n).data()[0];
    assert_eq!(scalar("a"), 1e-6);
    assert_eq!(get(&e, "b").data(), &[1e-8, 1e-9]);
    assert!(get(&e, "c").is_empty());
    assert_eq!(scalar("d"), 0.25);
    assert_eq!((scalar("q"), scalar("r"), scalar("x")), (1e-4, 2., 1e-2));
    assert_eq!((scalar("g"), scalar("h"), scalar("k")), (7., 3., 5.));
    // opts.RelTol reads the struct; updating one option keeps the others.
    assert_eq!(scalar("dot"), 1e-6);
    assert_eq!(get(&e, "kept").data(), &[1e-8, 1e-9]);
    assert_eq!(scalar("m"), 1e-6);
    // Any struct with the option fields works as an option set; the default has fewer steps.
    assert_eq!(get(&e, "ya").data(), get(&e, "yb").data());
    assert!(get(&e, "ta").rows() > get(&e, "tc").rows());
    assert_eq!((scalar("n"), scalar("em"), scalar("cls")), (22., 1., 1.));
    assert!(get(&e, "v").is_empty());
    let Some(Array::Struct(set)) = e.array("o") else {
        panic!("odeset returns a struct");
    };
    // Reference odeset field order; unset fields are [].
    let names: Vec<&str> = set.field_names().iter().map(String::as_str).collect();
    assert_eq!(names, ODESET_FIELDS);
    for (i, name) in names.iter().enumerate() {
        let unset = !matches!(*name, "AbsTol" | "RelTol");
        assert_eq!(set.get(i, 0).unwrap().is_empty(), unset, "{name}");
    }
    for (source, message) in [
        (
            "[t,y]=ode45('expdecay',[0 1],1,odeset('Events',1));",
            "ode45 option 'Events' is not supported yet",
        ),
        (
            "[t,y]=ode23('expdecay',[0 1],1,odeset('Mass',1));",
            "ode23 option 'Mass' is not supported yet",
        ),
        ("odeget(o,5)", "Property name must be a character vector."),
        ("odeget(1,2,3,4)", "Too many input arguments."),
    ] {
        let message_found = error(&mut e, source);
        assert!(message_found.contains(message), "{source}: {message_found}");
    }
}

#[test]
fn argument_errors_name_the_problem() {
    let (_folder, mut e) = engine();
    for (source, message) in [
        ("[t,y]=ode45();", "Not enough input arguments. See ODE45."),
        ("[t,y]=ode23();", "Not enough input arguments. See ODE23."),
        ("[t,y]=ode45('vdp');", "requires non-empty tspan and y0"),
        (
            "[t,y]=ode23('vdp',[0 1],[]);",
            "requires non-empty tspan and y0",
        ),
        (
            "[t,y]=ode45('rowout',[0 1],[1;2]);",
            "rowout must return a column vector.",
        ),
        (
            "[t,y]=ode45('longout',[0 1],[1;2]);",
            "longout returns a vector of length 3, but the length of initial conditions vector is 2.",
        ),
        (
            "[t,y]=ode45('vdp',[0 0],[1;2]);",
            "The last entry in tspan must be different",
        ),
        (
            "[t,y]=ode45('vdp',[0 2 1 3],[1;2]);",
            "must strictly increase or decrease",
        ),
        (
            "[t,y]=ode45('vdp',[0 1],[1;2],odeset('RelTol',-1));",
            "RelTol must be a positive scalar.",
        ),
        (
            "[t,y]=ode45('vdp',[0 1],[1;2],odeset('AbsTol',[1 2 3]));",
            "requires a scalar AbsTol",
        ),
        (
            "[t,y]=ode45('vdp',[0 1],[1;2],odeset('AbsTol',0));",
            "AbsTol must be positive.",
        ),
        (
            "[t,y]=ode45('vdp',[0 1],[1;2],odeset('MaxStep',0));",
            "Option 'MaxStep' must be greater than zero.",
        ),
        (
            "[t,y]=ode45('vdp',[0 1],[1;2],odeset('InitialStep',0));",
            "Option 'InitialStep' must be greater than zero.",
        ),
        (
            "[t,y]=ode45('vdp',[0 1],[1;2],5);",
            "options must be [] or an option set",
        ),
        (
            "[t,y]=ode45(5,[0 1],[1;2]);",
            "expects a derivative function handle or function name as text",
        ),
        (
            "[t,y]=ode45('nosuch',[0 1],1);",
            "Undefined function 'nosuch'",
        ),
        ("[t,y,te]=ode45('vdp',[0 1],[1;2]);", "at most two outputs"),
        (
            "[t,y]=ode45('vdp',[0 1],[true;false]);",
            "Invalid type for argument Y. Type should be double.",
        ),
    ] {
        let found = error(&mut e, source);
        assert!(found.contains(message), "{source}: {found}");
    }
    // Empty options mean defaults.
    e.execute("[t,y]=ode45('vdp',[0 1],[2;0],[]);[u,v]=ode45('vdp',[0 1],[2;0]);")
        .unwrap();
    assert_eq!(get(&e, "y").data(), get(&e, "v").data());
}

#[test]
fn non_finite_tspan_is_rejected_before_stepping() {
    let (_folder, mut e) = engine();
    for solver in ["ode45", "ode23"] {
        for tspan in [
            "[0 Inf]",
            "Inf",
            "[0 1 Inf]",
            "[0 -Inf]",
            "[0 NaN]",
            "[NaN 1]",
        ] {
            let source = format!("[t,y]={solver}('expdecay',{tspan},1);");
            let start = std::time::Instant::now();
            let found = error(&mut e, &source);
            assert_eq!(found, "The entries in tspan must be finite.", "{source}");
            assert!(
                start.elapsed().as_millis() < 200,
                "{source} took {:?}",
                start.elapsed()
            );
        }
    }
    e.execute("t=ode45('expdecay',Inf,1);").unwrap_err();
    assert!(!e.variables.contains_key("t"));
}

#[test]
fn tolerance_failure_warns_and_returns_the_solution_so_far() {
    let (_folder, mut e) = engine();
    let run = e.execute("[t,y]=ode45('blowup',[0 2],1);").unwrap();
    let t = get(&e, "t");
    assert!(
        run.output
            .iter()
            .any(|l| l.starts_with("Warning: Integration stopped at t=9.99")
                && l.contains("meeting the tolerances would require a step below the minimum"))
    );
    let run = e
        .execute("[t,y]=ode45('expdecay',[0 1],1,odeset('RelTol',1e-20));")
        .unwrap();
    assert_eq!(
        run.output,
        ["Warning: RelTol has been increased to 2.22045e-14."]
    );
    let last = *t.data().last().unwrap();
    assert!(last > 0.99 && last < 1., "{last}");
}

const CASES: [(&str, &str); 17] = [
    ("vdp45", "[t,y]=ode45('vdp',[0 20],[2;0]);"),
    ("vdp23", "[t,y]=ode23('vdp',[0 20],[2;0]);"),
    ("exp45", "[t,y]=ode45('expdecay',[0 5],1);"),
    ("exp23", "[t,y]=ode23('expdecay',[0 5],1);"),
    ("lin45", "[t,y]=ode45('lin2',[0 10],[1;0]);"),
    ("lin23", "[t,y]=ode23('lin2',[0 10],[1;0]);"),
    (
        "vdp45tol",
        "[t,y]=ode45('vdp',[0 20],[2;0],odeset('RelTol',1e-6,'AbsTol',1e-8));",
    ),
    (
        "vdp23tol",
        "[t,y]=ode23('vdp',[0 20],[2;0],odeset('RelTol',1e-6,'AbsTol',1e-8));",
    ),
    (
        "exp45max",
        "[t,y]=ode45('expdecay',[0 5],1,odeset('MaxStep',0.5));",
    ),
    (
        "lin45init",
        "[t,y]=ode45('lin2',[0 10],[1;0],odeset('InitialStep',0.01));",
    ),
    (
        "lin45r1",
        "[t,y]=ode45('lin2',[0 10],[1;0],odeset('Refine',1));",
    ),
    (
        "lin23r2",
        "[t,y]=ode23('lin2',[0 10],[1;0],odeset('Refine',2));",
    ),
    ("vdp45vec", "[t,y]=ode45('vdp',[0 1 2 5 10 20],[2;0]);"),
    ("exp23vec", "[t,y]=ode23('expdecay',linspace(0,5,11),1);"),
    ("exp45back", "[t,y]=ode45('expdecay',[1 0],1);"),
    ("lin45row", "[t,y]=ode45('lin2',[0 1],[1 0]);"),
    (
        "lin45abs",
        "[t,y]=ode45('lin2',[0 3],[1;0],odeset('AbsTol',[1e-6 1e-9],'RelTol',1e-5));",
    ),
];

fn reference_file(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ode")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Replays fn-ode-expected-call1.txt (ODE_ORACLE can name another copy) and checks
/// every t and y entry to 1e-10. `--nocapture` prints the max abs difference per case.
#[test]
fn reference_comparison() {
    let text = match std::env::var("ODE_ORACLE") {
        Ok(path) => std::fs::read_to_string(path).unwrap(),
        Err(_) => reference_file("fn-ode-expected-call1.txt"),
    };
    let (_folder, mut e) = engine();
    let mut worst_all: f64 = 0.;
    for (name, source) in CASES {
        let header = format!("CASE {name} ");
        let start = text
            .find(&header)
            .unwrap_or_else(|| panic!("{name} missing"));
        let mut lines = text[start..].lines();
        let dims: Vec<usize> = lines.next().unwrap()[header.len()..]
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        let rows: Vec<Vec<f64>> = lines
            .take(dims[0])
            .map(|l| l.split_whitespace().map(|v| v.parse().unwrap()).collect())
            .collect();
        e.execute(source).unwrap();
        let (t, y) = (get(&e, "t"), get(&e, "y"));
        assert_eq!(
            (t.rows(), y.cols()),
            (dims[0], dims[1]),
            "{name}: size MATLAB {}x{} MatRust {}x{}",
            dims[0],
            dims[1],
            y.rows(),
            y.cols()
        );
        let mut worst: f64 = 0.;
        for (i, row) in rows.iter().enumerate() {
            worst = worst.max((row[0] - t.data()[i]).abs());
            for c in 0..dims[1] {
                worst = worst.max((row[1 + c] - y.get(i, c)).abs());
            }
        }
        println!("{name}: {} points, max abs diff {worst:e}", dims[0]);
        assert!(worst <= 1e-10, "{name}: {worst:e}");
        worst_all = worst_all.max(worst);
    }
    println!("worst {worst_all:e}");
}

#[test]
fn handles_match_text_names_for_decay_and_vdp_reference_cases() {
    let (_folder, mut e) = engine();
    for (_, source) in CASES
        .iter()
        .filter(|(_, source)| source.contains("'expdecay'") || source.contains("'vdp'"))
    {
        e.execute(source).unwrap();
        let (t, y) = (get(&e, "t"), get(&e, "y"));
        let named = source
            .replace("'expdecay'", "@expdecay")
            .replace("'vdp'", "@vdp");
        let anonymous = source
            .replace("'expdecay'", "@(t,y) -y")
            .replace("'vdp'", "@(t,y) [y(2); (1-y(1)^2)*y(2)-y(1)]");
        for call in [named, anonymous] {
            e.execute(&call).unwrap();
            assert_eq!(get(&e, "t").data(), t.data(), "{call}");
            assert_eq!(get(&e, "y").data(), y.data(), "{call}");
            assert_eq!(
                (get(&e, "y").rows(), get(&e, "y").cols()),
                (y.rows(), y.cols())
            );
        }
    }
}

#[test]
fn anonymous_derivatives_capture_workspace_and_return_solution_structures() {
    let mut e = Engine::default();
    for solver in ["ode45", "ode23"] {
        e.execute(&format!(
            "k=3; f=@(t,y) -k*y; k=99; \
             [t,y]={solver}(f,[0 0.5 1],1,odeset('RelTol',1e-9,'AbsTol',1e-11));\
             [u,v]={solver}(@(t,y) -3*y,[0 0.5 1],1,odeset('RelTol',1e-9,'AbsTol',1e-11));\
             sol={solver}(f,[0 1],1); sx=sol.x; sy=sol.y; steps=sol.stats.nsteps;"
        ))
        .unwrap();
        assert_eq!(get(&e, "t").data(), get(&e, "u").data());
        assert_eq!(get(&e, "y").data(), get(&e, "v").data());
        for (&t, &y) in get(&e, "t").data().iter().zip(get(&e, "y").data()) {
            assert!((y - (-3. * t).exp()).abs() < 2e-9);
        }
        let (sx, sy) = (get(&e, "sx"), get(&e, "sy"));
        assert_eq!((sx.rows(), sy.rows(), sx.cols()), (1, 1, sy.cols()));
        assert_eq!(sx.cols(), get(&e, "steps").data()[0] as usize + 1);
        assert_eq!(*sx.data().last().unwrap(), 1.);
    }
}

/// MaxStep forces 2.25M accepted steps even at this equilibrium. The handle
/// returns 27,000,002 derivative elements, but the stored solution has only
/// 6,750,003 numeric elements. Previously their cumulative charge exceeded the
/// 32 Mi-element statement budget. Keep this stress test in release runs only.
#[test]
#[cfg_attr(debug_assertions, ignore = "long release budget regression")]
fn long_two_equation_handle_solve_charges_only_stored_output() {
    let mut e = Engine::default();
    let start = std::time::Instant::now();
    e.execute(
        "sol=ode45(@(t,y) y,[0 2250000],[0;0],\
         odeset('RelTol',1e-12,'AbsTol',1e-14,'InitialStep',1,'MaxStep',1));\
         nfevals=sol.stats.nfevals; sx=sol.x; sy=sol.y;",
    )
    .unwrap();
    assert_eq!(get(&e, "nfevals").data(), &[13_500_001.]);
    let x = get(&e, "sx");
    let y = get(&e, "sy");
    assert_eq!((x.rows(), x.cols()), (1, 2_250_001));
    assert_eq!(*x.data().last().unwrap(), 2_250_000.);
    assert_eq!((y.rows(), y.cols()), (2, 2_250_001));
    assert!(y.data().iter().all(|v| *v == 0.));
    println!("27M transient derivative elements: {:?}", start.elapsed());
}

#[test]
fn derivative_handle_errors_preserve_messages_and_matlab_size_diagnostics() {
    let reference = reference_file("fn-ode-expected-call2.txt");
    let mut e = Engine::default();
    for solver in ["ode45", "ode23"] {
        let found = error(
            &mut e,
            &format!("[t,y]={solver}(@(t,y) error('derivative failed'),[0 1],1);"),
        );
        assert!(found.contains("derivative failed"), "{found}");
        for body in ["y.'", "[y;1]"] {
            let prefix = format!("ERR ode45(@(t,y) {body},[0 1],[1;2]) => ");
            let expected = reference
                .lines()
                .find_map(|line| line.strip_prefix(&prefix))
                .unwrap();
            let found = error(
                &mut e,
                &format!("[t,y]={solver}(@(t,y) {body},[0 1],[1;2]);"),
            );
            assert_eq!(found, expected);
        }
        for args in ["", ",[0 1]", ",[0 1],[]"] {
            assert_eq!(
                error(&mut e, &format!("[t,y]={solver}(@(t,y) -y{args});")),
                format!(
                    "When the first argument to {solver} is a function handle, the tspan and y0 arguments must be supplied."
                )
            );
        }
    }
}

#[test]
fn odeset_keeps_event_handles_and_solvers_explicitly_reject_events() {
    let mut e = Engine::default();
    e.execute("ev=@(t,y) y; o=odeset('Events',ev); f=odeget(o,'Events'); v=f(0,7);")
        .unwrap();
    assert_eq!(get(&e, "v").data(), &[7.]);
    for solver in ["ode45", "ode23"] {
        assert_eq!(
            error(&mut e, &format!("[t,y]={solver}(@(t,y) -y,[0 1],1,o);")),
            format!(
                "{solver} option 'Events' is not supported yet; supported: AbsTol, InitialStep, MaxStep, Refine, RelTol"
            )
        );
    }
}

/// Reference call2.m error cases: (call2.m expression, MatRust source, MatRust function name
/// standing in for the anonymous function the reference names in its message).
const ERROR_CASES: [(&str, &str, &str); 24] = [
    ("ode45()", "[t,y]=ode45();", ""),
    (
        "ode45(@(t,y) y.',[0 1],[1;2])",
        "[t,y]=ode45('rowout',[0 1],[1;2]);",
        "@(T,Y)Y.'|rowout",
    ),
    (
        "ode45(@(t,y) [y;1],[0 1],[1;2])",
        "[t,y]=ode45('longout',[0 1],[1;2]);",
        "@(T,Y)[Y;1]|longout",
    ),
    ("ode45(fe,[0 0],1)", "[t,y]=ode45('expdecay',[0 0],1);", ""),
    (
        "ode45(fe,[0 2 1 3],1)",
        "[t,y]=ode45('expdecay',[0 2 1 3],1);",
        "",
    ),
    (
        "ode45(fe,[0 1],1,odeset('RelTol',-1))",
        "[t,y]=ode45('expdecay',[0 1],1,odeset('RelTol',-1));",
        "",
    ),
    (
        "ode45(fe,[0 1],[1;2],odeset('AbsTol',[1 2 3]))",
        "[t,y]=ode45('expdecay',[0 1],[1;2],odeset('AbsTol',[1 2 3]));",
        "@(T,Y)-Y|expdecay",
    ),
    (
        "ode45(fe,[0 1],1,odeset('AbsTol',0))",
        "[t,y]=ode45('expdecay',[0 1],1,odeset('AbsTol',0));",
        "",
    ),
    (
        "ode45(fe,[0 1],1,odeset('MaxStep',0))",
        "[t,y]=ode45('expdecay',[0 1],1,odeset('MaxStep',0));",
        "",
    ),
    (
        "ode45(fe,[0 1],1,odeset('InitialStep',0))",
        "[t,y]=ode45('expdecay',[0 1],1,odeset('InitialStep',0));",
        "",
    ),
    (
        "ode45(fe,[0 1],[true;false])",
        "[t,y]=ode45('expdecay',[0 1],[true;false]);",
        "",
    ),
    ("odeset('Foo',1)", "o=odeset('Foo',1);", ""),
    ("odeset('Max',1)", "o=odeset('Max',1);", ""),
    ("odeset('RelTol')", "o=odeset('RelTol');", ""),
    ("odeset('RelTol',1,2,3)", "o=odeset('RelTol',1,2,3);", ""),
    ("odeset(5)", "o=odeset(5);", ""),
    ("odeget(odeset(),'Foo')", "v=odeget(odeset(),'Foo');", ""),
    ("odeget(5,'RelTol')", "v=odeget(5,'RelTol');", ""),
    ("odeget(odeset())", "v=odeget(odeset());", ""),
    (
        "odeget(odeset('Max',1),'Max')",
        "v=odeget(odeset('Max',1),'Max');",
        "",
    ),
    ("ode45(fe)", "[t,y]=ode45('expdecay');", "skip"),
    ("ode45(fe,[0 1])", "[t,y]=ode45('expdecay',[0 1]);", "skip"),
    (
        "ode45(fe,[0 1],[])",
        "[t,y]=ode45('expdecay',[0 1],[]);",
        "skip",
    ),
    (
        "ode45(fe,[0 NaN],1)",
        "[t,y]=ode45('expdecay',[0 NaN],1);",
        "skip",
    ),
];

/// Every reference error in fn-ode-expected-call2.txt that applies
/// to an ODE-file name equals MatRust's, with the function name in place of the anonymous
/// function text. The four skipped cases need function handles (message about handles) or
/// show MATLAB plotting for nargout=0 (NaN tspan).
#[test]
fn reference_error_texts() {
    let text = reference_file("fn-ode-expected-call2.txt");
    let (_folder, mut e) = engine();
    let mut compared = 0;
    for (expr, source, swap) in ERROR_CASES {
        if swap == "skip" {
            continue;
        }
        let prefix = format!("ERR {expr} => ");
        let line = text
            .lines()
            .find_map(|l| l.strip_prefix(prefix.as_str()))
            .unwrap_or_else(|| panic!("{expr}: no ERR line"));
        let expected = match swap.split_once('|') {
            Some((from, to)) => line.replace(from, to),
            None => line.to_string(),
        };
        let found = error(&mut e, source);
        assert_eq!(found, expected, "{expr}");
        compared += 1;
    }
    assert_eq!(compared, 20);
}

/// The non-error part of the reference call2.m transcript: odeget defaults, ODE-file name forms, warnings.
#[test]
fn reference_semantics() {
    let (_folder, mut e) = engine();
    let text = reference_file("fn-ode-expected-call2.txt");
    let run = e
        .execute(
            "o=odeset('reltol',1e-6,'AbsTol',[1e-8 1e-9]);a=odeget(o,'RelTol');b=odeget(o,'abs');\
             c=1*isempty(odeget(o,'MaxStep'));d=odeget(o,'MaxStep',0.25);\
             one=ode45('expdecay',[0 2],1);[u,v]=ode45('expdecay',2,1);\
             [t,y]=ode45('blowup',[0 2],1);\
             [t2,y2]=ode45('expdecay',[0 1],1,odeset('RelTol',1e-20));\
             [t3,y3]=ode45('expdecay',[0 1],NaN);",
        )
        .unwrap();
    let scalar = |n: &str| get(&e, n).data()[0];
    assert!(text.contains("odeget 1e-06 1e-08 1e-09 | 1 | 0.25"));
    assert_eq!((scalar("a"), scalar("c"), scalar("d")), (1e-6, 1., 0.25));
    assert_eq!(get(&e, "b").data(), &[1e-8, 1e-9]);
    // "name one output: 41 x 1" and "scalar tspan: 41 x 1, u(end)=2"
    assert_eq!((get(&e, "one").rows(), get(&e, "one").cols()), (41, 1));
    assert_eq!((get(&e, "u").rows(), get(&e, "u").cols()), (41, 1));
    assert_eq!(*get(&e, "u").data().last().unwrap(), 2.);
    // "blowup: t(end)=0.99996942883098183 n=265"
    let t = get(&e, "t");
    assert_eq!(t.rows(), 265);
    assert!((t.data()[264] - 0.999_969_428_830_981_8).abs() < 1e-14);
    // Same stopping time and minimum step as the reference, with MatRust's wording.
    assert_eq!(
        run.output,
        [
            "Warning: Integration stopped at t=9.999694e-01; meeting the tolerances would require a step below the minimum (1.776357e-15).",
            "Warning: RelTol has been increased to 2.22045e-14."
        ]
    );
    // "NaN y0: 41 x 1 last NaN"
    let y3 = get(&e, "y3");
    assert_eq!((y3.rows(), y3.cols()), (41, 1));
    assert!(y3.data()[40].is_nan());
}

/// Writes MatRust's results for CASES in the reference format:
/// `ODE_MATRUST_OUT=out.txt cargo test --release --test ode -- --ignored --nocapture case_dump`.
#[test]
#[ignore]
fn case_dump() {
    let path = std::env::var("ODE_MATRUST_OUT").expect("set ODE_MATRUST_OUT");
    let (_folder, mut e) = engine();
    let mut text = String::new();
    for (name, source) in CASES {
        e.execute(source).unwrap();
        let (t, y) = (get(&e, "t"), get(&e, "y"));
        text.push_str(&format!("CASE {name} {} {}\n", y.rows(), y.cols()));
        for i in 0..t.rows() {
            text.push_str(&format!("{:e}", t.data()[i]));
            for c in 0..y.cols() {
                text.push_str(&format!(" {:e}", y.get(i, c)));
            }
            text.push('\n');
        }
        println!("{name}: {}x{}", y.rows(), y.cols());
    }
    std::fs::write(path, text).unwrap();
    e.execute("[t,y]=ode45('blowup',[0 2],1);").unwrap();
    let t = get(&e, "t");
    println!("blowup: t(end)={:e} n={}", t.data()[t.len() - 1], t.len());
    e.execute("[t,y]=ode45('expdecay',[0 1],NaN);[u,v]=ode45('expdecay',2,1);")
        .unwrap();
    let (y, u) = (get(&e, "y"), get(&e, "u"));
    println!(
        "NaN y0: {}x{} last {}; scalar tspan {} points, u(end)={:e}",
        y.rows(),
        y.cols(),
        y.data()[y.len() - 1],
        u.len(),
        u.data()[u.len() - 1]
    );
}

#[test]
fn names_are_registered_with_help_pages() {
    for name in ["ode45", "ode23", "odeset", "odeget"] {
        assert!(matrust::session::FUNCTIONS.contains(&name));
        assert!(matrust::help::summary(name).is_some(), "{name}");
    }
}

#[test]
fn help_examples_print_their_documented_output() {
    for solver in ["ode45", "ode23"] {
        let mut fresh = Engine::default();
        let run = fresh
            .execute(&format!("[t, y] = {solver}(@(t,y) -y, [0 1], 1); y(end)"))
            .unwrap();
        assert!(run.output.join("\n").contains("0.3679"));
    }
    let mut e = Engine::default();
    e.execute(
        "opts = odeset('RelTol', 1e-6, 'AbsTol', 1e-8); g = odeget(odeset('RelTol', 1e-6), 'RelTol');\
         [t, y] = ode45(@(t,y) -y, [0 1 2], 1);",
    )
    .unwrap();
    assert_eq!(get(&e, "g").data(), &[1e-6]);
    assert_eq!(get(&e, "t").data(), &[0., 1., 2.]);
    let y = get(&e, "y");
    assert!((y.data()[1] - 0.367879).abs() < 5e-7 && (y.data()[2] - 0.135335).abs() < 5e-7);
    assert!(matches!(e.array("opts"), Some(Array::Struct(_))));
}
