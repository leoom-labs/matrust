//! lyap/dlyap, pzmap, pid/pidstd and pidtune.
//! Equation references: SciPy (tests/fixtures/control/lyap.json, regenerate with
//! bash tool/ref-python.sh tool/refs/control/make_refs.py). PID transfer
//! functions are worked out by hand from the textbook formulas.
use matrust::{Array, Engine};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    origin: String,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Mat {
    rows: usize,
    cols: usize,
    data: Vec<f64>,
}
#[derive(Deserialize)]
#[allow(non_snake_case)]
struct Case {
    kind: String,
    A: Mat,
    B: Option<Mat>,
    C: Option<Mat>,
    Q: Option<Mat>,
    X: Mat,
}
fn literal(m: &Mat) -> String {
    let rows: Vec<String> = (0..m.rows)
        .map(|i| {
            (0..m.cols)
                .map(|j| format!("{:?}", m.data[i + j * m.rows]))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    format!("[{}]", rows.join(";"))
}
fn real(e: &Engine, name: &str) -> (usize, usize, Vec<f64>) {
    match e.array(name).unwrap_or_else(|| panic!("missing {name}")) {
        Array::Numeric(m) => (m.rows(), m.cols(), m.data().to_vec()),
        Array::Logical(m) => (
            m.rows(),
            m.cols(),
            m.data().iter().map(|&b| f64::from(b)).collect(),
        ),
        other => panic!("{name} is not real numeric: {}", other.class()),
    }
}
fn complex(e: &Engine, name: &str) -> Vec<(f64, f64)> {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m.data().iter().map(|&x| (x, 0.)).collect(),
        Array::Complex(m) => m.data().iter().map(|z| (z.re, z.im)).collect(),
        _ => panic!("{name} is not numeric"),
    }
}
fn number(e: &Engine, name: &str) -> f64 {
    real(e, name).2[0]
}
fn text(e: &Engine, name: &str) -> String {
    let Array::Character(c) = e.array(name).unwrap() else {
        panic!("{name} is not char")
    };
    String::from_utf16_lossy(c.units())
}
fn norm(v: &[f64]) -> f64 {
    v.iter().fold(0_f64, |s, x| s.hypot(*x))
}
fn error(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(_) => panic!("accepted: {code}"),
        Err(err) => err.to_string(),
    }
}
fn same_set(actual: &[(f64, f64)], expected: &[(f64, f64)], tol: f64) {
    assert_eq!(actual.len(), expected.len(), "{actual:?} vs {expected:?}");
    let mut left = actual.to_vec();
    for z in expected {
        let i = left
            .iter()
            .position(|a| (a.0 - z.0).hypot(a.1 - z.1) < tol)
            .unwrap_or_else(|| panic!("{z:?} missing from {actual:?}"));
        left.remove(i);
    }
}

#[test]
fn lyapunov_sylvester_and_stein_match_scipy() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/control/lyap.json")).unwrap();
    assert!(fixture.origin.contains("SciPy"));
    let mut counts = std::collections::BTreeMap::new();
    for case in &fixture.cases {
        *counts.entry(case.kind.as_str()).or_insert(0) += 1;
        let a = literal(&case.A);
        let (call, residual) = match case.kind.as_str() {
            "lyap" => (
                format!("Q={}; X=lyap(A,Q);", literal(case.Q.as_ref().unwrap())),
                "R=A*X+X*A'+Q; s=norm(A,'fro')^2*norm(X,'fro')+norm(Q,'fro');",
            ),
            "dlyap" => (
                format!("Q={}; X=dlyap(A,Q);", literal(case.Q.as_ref().unwrap())),
                "R=A*X*A'-X+Q; s=norm(A,'fro')^2*norm(X,'fro')+norm(X,'fro')+norm(Q,'fro');",
            ),
            "sylvester" => (
                format!(
                    "B={}; C={}; X=lyap(A,B,C);",
                    literal(case.B.as_ref().unwrap()),
                    literal(case.C.as_ref().unwrap())
                ),
                "R=A*X+X*B+C; s=(norm(A,'fro')+norm(B,'fro'))*norm(X,'fro')+norm(C,'fro');",
            ),
            _ => (
                format!(
                    "B={}; C={}; X=dlyap(A,B,C);",
                    literal(case.B.as_ref().unwrap()),
                    literal(case.C.as_ref().unwrap())
                ),
                "R=A*X*B-X+C; s=norm(A,'fro')*norm(B,'fro')*norm(X,'fro')+norm(X,'fro')+norm(C,'fro');",
            ),
        };
        let code = format!("A={a}; {call} {residual} r=norm(R,'fro');");
        let mut e = Engine::default();
        e.execute(&code)
            .unwrap_or_else(|err| panic!("{}: {err}", case.kind));
        let (rows, cols, x) = real(&e, "X");
        assert_eq!((rows, cols), (case.X.rows, case.X.cols), "{}", case.kind);
        let difference: Vec<f64> = x.iter().zip(&case.X.data).map(|(a, b)| a - b).collect();
        assert!(
            norm(&difference) <= 1e-10 * norm(&case.X.data),
            "{}: relative error {}",
            case.kind,
            norm(&difference) / norm(&case.X.data)
        );
        assert!(
            number(&e, "r") < 1e-12 * number(&e, "s"),
            "{}: residual {} scale {}",
            case.kind,
            number(&e, "r"),
            number(&e, "s")
        );
    }
    for kind in ["lyap", "sylvester", "dlyap", "dsylvester"] {
        assert!(counts[kind] >= 3, "{kind}");
    }
}

#[test]
fn lyapunov_singular_and_invalid_inputs() {
    let mut e = Engine::default();
    // Eigenvalues +1 and -1: lambda_1 + lambda_2 = 0.
    let message = error(&mut e, "X=lyap([1 0;0 -1],eye(2));");
    assert!(
        message.contains("lambda_i(A) + lambda_j(B) = 0"),
        "{message}"
    );
    let message = error(&mut e, "X=lyap(2,-2,1);");
    assert!(message.contains("no unique solution"), "{message}");
    // Eigenvalues 2 and 0.5: lambda_1 * lambda_2 = 1.
    let message = error(&mut e, "X=dlyap([2 0;0 0.5],eye(2));");
    assert!(
        message.contains("lambda_i(A) * lambda_j(B) = 1"),
        "{message}"
    );
    let message = error(&mut e, "X=dlyap(1,eye(1));");
    assert!(message.contains("no unique solution"), "{message}");
    for (code, needle) in [
        ("X=lyap([1 2 3;4 5 6],eye(2));", "square"),
        ("X=lyap(-eye(2),eye(3));", "Q must be 2 by 2"),
        ("X=lyap(-eye(2),-eye(3),ones(3,2));", "C must be 2 by 3"),
        ("X=lyap(-eye(2),[1 2],ones(2,2));", "square"),
        ("X=dlyap(zeros(2,3),eye(2));", "square"),
        ("X=dlyap(0.5*eye(2),eye(3));", "Q must be 2 by 2"),
        ("X=dlyap(0.5*eye(2),0.5,ones(1,2));", "C must be 2 by 1"),
        ("X=lyap(-eye(2),eye(2),[],eye(2));", "not supported yet"),
        ("X=dlyap(0.5*eye(2),eye(2),[],eye(2));", "not supported yet"),
        ("X=lyap([]);", "requires"),
    ] {
        let message = error(&mut e, code);
        assert!(message.contains(needle), "{code}: {message}");
    }
}

#[test]
fn pzmap_returns_roots_without_plotting() {
    let mut e = Engine::default();
    e.execute("G=tf([1 3 2],[1 2 5 0]); [p,z]=pzmap(G); Z=zpk([-4],[-1+2i -1-2i -5],3); [p2,z2]=pzmap(Z); p3=pzmap(G);")
        .unwrap();
    assert!(e.figure.is_none(), "outputs requested: no plot");
    same_set(&complex(&e, "p"), &[(0., 0.), (-1., 2.), (-1., -2.)], 1e-12);
    same_set(&complex(&e, "z"), &[(-1., 0.), (-2., 0.)], 1e-12);
    same_set(
        &complex(&e, "p2"),
        &[(-1., 2.), (-1., -2.), (-5., 0.)],
        1e-12,
    );
    same_set(&complex(&e, "z2"), &[(-4., 0.)], 1e-12);
    assert_eq!(complex(&e, "p3").len(), 3);
    assert!(e.execute("[p,z]=pzmap(G,G);").is_err());
    assert!(e.execute("pzmap(1);").is_err());
}

fn series_points(s: &matrust::plot::Series) -> Vec<(f64, f64)> {
    s.points.iter().map(|p| (p[0], p[1])).collect()
}

#[test]
fn pzmap_draws_pole_and_zero_markers() {
    use matrust::plot::{LinePattern, MarkerShape};
    let mut e = Engine::default();
    e.execute("G=zpk([-4],[-1+2i -1-2i -5],3); pzmap(G);")
        .unwrap();
    let f = e.figure.as_ref().expect("figure");
    let axes: Vec<_> = f.all_axes().collect();
    assert_eq!(axes.len(), 1);
    let a = axes[0].1;
    let crosses: Vec<_> = a
        .series
        .iter()
        .filter(|s| s.markers && s.effective_marker() == MarkerShape::Cross)
        .collect();
    let circles: Vec<_> = a
        .series
        .iter()
        .filter(|s| s.markers && s.effective_marker() == MarkerShape::Circle)
        .collect();
    assert_eq!((crosses.len(), circles.len()), (1, 1));
    same_set(
        &series_points(crosses[0]),
        &[(-1., 2.), (-1., -2.), (-5., 0.)],
        1e-12,
    );
    same_set(&series_points(circles[0]), &[(-4., 0.)], 1e-12);
    assert_eq!(crosses[0].kind, matrust::plot::PlotKind::Scatter);
    let dashed = a
        .series
        .iter()
        .filter(|s| s.pattern == LinePattern::Dashed)
        .count();
    assert_eq!(dashed, 2, "real and imaginary axis lines");
    assert!(
        !a.series.iter().any(|s| s.pattern == LinePattern::Dotted),
        "continuous: no unit circle"
    );
    assert_eq!(a.title, "Pole-Zero Map");
    assert_eq!(a.xlabel, "Real Axis");
    assert_eq!(a.ylabel, "Imaginary Axis");

    // Discrete: unit circle; several systems overlay; explicit axes.
    let mut e = Engine::default();
    e.execute("D=tf([1 -0.5],[1 -0.2 0.5],0.1); C=tf(1,[1 1]); ax=gca; pzmap(ax,D,C);")
        .unwrap();
    let f = e.figure.as_ref().expect("figure");
    let a = f.all_axes().next().unwrap().1;
    let circle: Vec<_> = a
        .series
        .iter()
        .filter(|s| s.pattern == LinePattern::Dotted)
        .collect();
    assert_eq!(circle.len(), 1);
    assert!(
        series_points(circle[0])
            .iter()
            .all(|(x, y)| (x.hypot(*y) - 1.).abs() < 1e-12)
    );
    let crosses = a
        .series
        .iter()
        .filter(|s| s.markers && s.effective_marker() == MarkerShape::Cross)
        .count();
    assert_eq!(crosses, 2, "poles of both systems");
}

#[test]
fn pid_objects_construct_convert_and_display() {
    let mut e = Engine::default();
    e.execute(
        "C1=pid(2); C2=pid(2,3); C3=pid(2,3,4); C4=pid(2,3,4,0.5); C5=pid(2,3,4,0.5,0.1); \
         S1=pidstd(2); S2=pidstd(2,4); S3=pidstd(2,4,0.5); S4=pidstd(2,4,0.5,10); S5=pidstd(2,4,0.5,10,0.2); \
         k=C4.Kp; i=C4.Ki; d=C4.Kd; f=C4.Tf; t=C5.Ts; ti=S4.Ti; td=S4.Td; n=S4.N; ts=S5.Ts; \
         c1=class(C4); c2=class(S4); a1=isa(C4,'pid'); a2=isa(S4,'lti'); a3=isa(C4,'tf'); \
         fi=C5.IFormula; fd=C5.DFormula; fc=C4.IFormula;",
    )
    .unwrap();
    for (name, value) in [
        ("k", 2.),
        ("i", 3.),
        ("d", 4.),
        ("f", 0.5),
        ("t", 0.1),
        ("ti", 4.),
        ("td", 0.5),
        ("n", 10.),
        ("ts", 0.2),
        ("a1", 1.),
        ("a2", 1.),
        ("a3", 0.),
    ] {
        assert_eq!(number(&e, name), value, "{name}");
    }
    assert_eq!(text(&e, "c1"), "pid");
    assert_eq!(text(&e, "c2"), "pidstd");
    assert_eq!(text(&e, "fi"), "ForwardEuler");
    assert_eq!(text(&e, "fd"), "ForwardEuler");
    assert_eq!(text(&e, "fc"), "");

    // Property assignment is validated and keeps the object.
    e.execute("C=C4; C.Ki=7; ki=C.Ki; old=C4.Ki; cc=class(C); C.Ts=0.5; f2=C.IFormula;")
        .unwrap();
    assert_eq!(number(&e, "ki"), 7.);
    assert_eq!(number(&e, "old"), 3.);
    assert_eq!(text(&e, "cc"), "pid");
    assert_eq!(text(&e, "f2"), "ForwardEuler");
    for code in [
        "C=C4; C.Tf=-1;",
        "C=C4; C.Kp=NaN;",
        "C=S4; C.Ti=0;",
        "C=C4; C.Missing=1;",
        "C=C5; C.IFormula='Trapezoidal';",
        "C=pid(1,2,0,0,0.1,'IFormula','BackwardEuler');",
        "C=pid([1 2]);",
        "C=pid('x');",
    ] {
        assert!(e.execute(code).is_err(), "accepted {code}");
    }
    let message = error(&mut e, "C=pid(1,2,0,0,0.1,'IFormula','Trapezoidal');");
    assert!(message.contains("not supported yet"), "{message}");

    // Display: formula, gains and form.
    for (name, markers) in [
        ("C1", vec!["Kp = 2", "P-only controller in parallel form"]),
        (
            "C4",
            vec![
                "Kp + Ki * 1/s + Kd * s/(Tf*s+1)",
                "with Kp = 2, Ki = 3, Kd = 4, Tf = 0.5",
                "Continuous-time PIDF controller in parallel form.",
            ],
        ),
        ("C2", vec!["Kp + Ki * 1/s", "Continuous-time PI controller"]),
        (
            "C5",
            vec![
                "Kp + Ki * Ts/(z-1) + Kd * 1/(Tf+Ts/(z-1))",
                "Sample time: 0.1 seconds",
                "Discrete-time PIDF controller",
            ],
        ),
        (
            "S4",
            vec![
                "Kp * (1 + 1/Ti * 1/s + Td * s/((Td/N)*s+1))",
                "with Kp = 2, Ti = 4, Td = 0.5, N = 10",
                "PIDF controller in standard form",
            ],
        ),
    ] {
        let output = e.execute(name).unwrap().output.join("\n");
        assert!(output.starts_with(&format!("{name} =")), "{output}");
        for marker in markers {
            assert!(output.contains(marker), "{marker}: {output}");
        }
        assert!(!output.contains("MatRustLTI"));
    }
}

#[test]
fn pid_transfer_functions_by_hand() {
    let mut e = Engine::default();
    // Expected coefficients (worked by hand):
    // pid(2,3,4,0.5): 2 + 3/s + 4s/(0.5s+1) = (5s^2 + 3.5s + 3)/(0.5s^2 + s)
    //   = (10s^2 + 7s + 6)/(s^2 + 2s).
    // pid(2,3,4): (4s^2 + 2s + 3)/s.   pid(2,3): (2s + 3)/s.   pid(2): 2.
    // pidstd(2,4,0.5,10) = pid(2,0.5,1,0.05):
    //   2 + 0.5/s + s/(0.05s+1) = (1.1s^2 + 2.025s + 0.5)/(0.05s^2 + s)
    //   = (22s^2 + 40.5s + 10)/(s^2 + 20s).
    // pid(2,3,0,0,0.1): 2 + 0.3/(z-1) = (2z - 1.7)/(z - 1).
    // pid(2,0,4,0,0.1): 2 + 40(z-1) = (40z - 38)/1.
    // pid(1,0,4,0.5,0.1): 1 + 4(z-1)/(0.5z - 0.4) = (4.5z - 4.4)/(0.5z - 0.4)
    //   = (9z - 8.8)/(z - 0.8).
    for (ctor, num, den) in [
        ("pid(2,3,4,0.5)", vec![10., 7., 6.], vec![1., 2., 0.]),
        ("pid(2,3,4)", vec![4., 2., 3.], vec![1., 0.]),
        ("pid(2,3)", vec![2., 3.], vec![1., 0.]),
        ("pid(2)", vec![2.], vec![1.]),
        (
            "pidstd(2,4,0.5,10)",
            vec![22., 40.5, 10.],
            vec![1., 20., 0.],
        ),
        ("pid(2,3,0,0,0.1)", vec![2., -1.7], vec![1., -1.]),
        ("pid(2,0,4,0,0.1)", vec![40., -38.], vec![1.]),
        ("pid(1,0,4,0.5,0.1)", vec![9., -8.8], vec![1., -0.8]),
    ] {
        let mut padded = vec![0.; den.len().saturating_sub(num.len())];
        padded.extend(num);
        e.execute(&format!(
            "C={ctor}; G=tf(C); n=G.Numerator{{1}}; d=G.Denominator{{1}}; c=class(G);"
        ))
        .unwrap_or_else(|err| panic!("{ctor}: {err}"));
        assert_eq!(text(&e, "c"), "tf");
        for (name, expected) in [("n", &padded), ("d", &den)] {
            let (_, _, actual) = real(&e, name);
            assert_eq!(actual.len(), expected.len(), "{ctor} {name}: {actual:?}");
            for (a, b) in actual.iter().zip(expected.iter()) {
                assert!(
                    (a - b).abs() < 1e-12 * (1. + b.abs()),
                    "{ctor} {name}: {actual:?}"
                );
            }
        }
    }
    // zpk/ss conversions and the pid <-> pidstd round trip.
    e.execute(
        "C=pid(2,3,4,0.5); Z=zpk(C); z=class(Z); S=ss(C); s=class(S); p=sort(real(pole(C))); \
         Cs=pidstd(C); kp=Cs.Kp; ti=Cs.Ti; td=Cs.Td; nn=Cs.N; Cp=pid(Cs); ki=Cp.Ki; kd=Cp.Kd; tf2=Cp.Tf;",
    )
    .unwrap();
    assert_eq!(text(&e, "z"), "zpk");
    assert_eq!(text(&e, "s"), "ss");
    let (_, _, p) = real(&e, "p");
    assert!((p[0] + 2.).abs() < 1e-12 && p[1].abs() < 1e-12, "{p:?}");
    for (name, value) in [
        ("kp", 2.),
        ("ti", 2. / 3.),
        ("td", 2.),
        ("nn", 4.),
        ("ki", 3.),
        ("kd", 4.),
        ("tf2", 0.5),
    ] {
        assert!((number(&e, name) - value).abs() < 1e-12, "{name}");
    }
}

#[test]
fn pid_in_loops_matches_its_transfer_function() {
    let mut e = Engine::default();
    e.execute(
        "C=pid(2,1,0.5,0.1); G=tf(1,[1 2 1]); T1=feedback(C*G,1); T2=feedback(tf(C)*G,1); \
         n1=T1.Numerator{1}; d1=T1.Denominator{1}; n2=T2.Numerator{1}; d2=T2.Denominator{1}; \
         [y,t]=step(T1,0:0.1:5); S=series(C,G); s1=S.Numerator{1}; L=G*C; l1=L.Denominator{1}; \
         [gm,pm,wcg,wcp]=margin(C*G); [mag,ph,w]=bode(C,[0.1 1 10]); \
         Cd=pid(1,0.5,0,0,0.1); Gd=c2d(G,0.1); Td=feedback(Cd*Gd,1); [yd,td]=step(Td,0:0.1:2);",
    )
    .unwrap();
    for (a, b) in [("n1", "n2"), ("d1", "d2")] {
        let (_, _, x) = real(&e, a);
        let (_, _, y) = real(&e, b);
        assert_eq!(x.len(), y.len());
        for (p, q) in x.iter().zip(&y) {
            assert!((p - q).abs() < 1e-12 * (1. + q.abs()), "{a} {x:?} {y:?}");
        }
    }
    let (rows, _, y) = real(&e, "y");
    assert_eq!(rows, 51);
    // Type-1 loop: unit step tracked with zero steady-state error.
    assert!((y[50] - 1.).abs() < 0.05, "{}", y[50]);
    assert!(number(&e, "pm") > 0.);
    let (_, _, mag) = real(&e, "mag");
    // |C(j*1)| for 2 + 1/s + 0.5s/(0.1s+1) at s = j.
    let s = matrust::Complex64::new(0., 1.);
    let c = 2. + 1. / s + 0.5 * s / (0.1 * s + 1.);
    assert!((mag[1] - c.norm()).abs() < 1e-10, "{mag:?}");
    assert!(real(&e, "yd").2.iter().all(|v| v.is_finite()));
}

fn pidtune_check(e: &mut Engine, plant: &str, kind: &str) {
    e.execute(&format!(
        "G={plant}; [C,info]=pidtune(G,'{kind}'); wc=info.CrossoverFrequency; pm=info.PhaseMargin; \
         st=info.Stable; h=freqresp(C*G,wc); mg=abs(h); ph=angle(h)*180/pi; cl=class(C); \
         ok=isstable(feedback(C*G,1)); kp=C.Kp; ki=C.Ki; kd=C.Kd; tfv=C.Tf;"
    ))
    .unwrap_or_else(|err| panic!("{plant} {kind}: {err}"));
    let wc = number(e, "wc");
    assert!(wc > 0., "{plant} {kind}");
    assert!(
        (number(e, "mg") - 1.).abs() < 1e-9,
        "{plant} {kind}: |CG| = {}",
        number(e, "mg")
    );
    assert!(
        (number(e, "ph") + 120.).abs() < 1e-6,
        "{plant} {kind}: phase {}",
        number(e, "ph")
    );
    assert!(
        (number(e, "pm") - 60.).abs() < 1e-6,
        "{plant} {kind}: pm {}",
        number(e, "pm")
    );
    assert_eq!(number(e, "st"), 1.);
    assert_eq!(number(e, "ok"), 1., "{plant} {kind}");
    assert_eq!(text(e, "cl"), "pid");
    let (kp, ki, kd, tf) = (
        number(e, "kp"),
        number(e, "ki"),
        number(e, "kd"),
        number(e, "tfv"),
    );
    assert!(kp > 0.);
    if kind.contains('I') {
        assert!(
            (ki - kp * wc / 10.).abs() < 1e-12 * kp.max(1.),
            "Ki/Kp = wc/10"
        );
    } else {
        assert_eq!(ki, 0.);
    }
    if kind.contains('D') {
        assert!(kd > 0.);
    } else {
        assert_eq!(kd, 0.);
    }
    if kind == "PIDF" {
        assert!((tf - 1. / (10. * wc)).abs() < 1e-15 / wc.min(1.));
    } else {
        assert_eq!(tf, 0.);
    }
}

#[test]
fn pidtune_reaches_unit_gain_and_sixty_degree_margin() {
    let mut e = Engine::default();
    for plant in ["tf(1,[1 3 3 1])", "tf(1,[1 2 0])"] {
        for kind in ["P", "PI", "PD", "PID", "PIDF"] {
            pidtune_check(&mut e, plant, kind);
        }
    }
    // The default PID crossover is where the plant phase is -150 degrees:
    // for 1/(s+1)^3, 3*atan(w) = 150 deg, w = tan(50 deg).
    e.execute("[C,info]=pidtune(tf(1,[1 3 3 1]),'PID'); w=info.CrossoverFrequency;")
        .unwrap();
    assert!((number(&e, "w") - 50_f64.to_radians().tan()).abs() < 1e-9);
    // An explicit crossover frequency is honoured.
    e.execute("[C,info]=pidtune(tf(1,[1 2 0]),'PID',1.5); w=info.CrossoverFrequency; p=info.PhaseMargin; h=abs(freqresp(C*tf(1,[1 2 0]),1.5));")
        .unwrap();
    assert_eq!(number(&e, "w"), 1.5);
    assert!((number(&e, "p") - 60.).abs() < 1e-6);
    assert!((number(&e, "h") - 1.).abs() < 1e-9);
    // One output and statement form.
    e.execute("C=pidtune(tf(1,[1 3 3 1]),'PI');").unwrap();
}

#[test]
fn pidtune_reports_unstable_and_invalid_requests() {
    let mut e = Engine::default();
    // 1/(s^2-1): phase is -180 deg everywhere; P control can only move the
    // poles onto the imaginary axis, so the rule cannot stabilise it.
    let message = error(&mut e, "[C,info]=pidtune(tf(1,[1 0 -1]),'P');");
    assert!(message.contains("unstable"), "{message}");
    let message = error(&mut e, "[C,info]=pidtune(tf(1,[1 0 -1]),'PI');");
    assert!(message.contains("unstable"), "{message}");
    for (code, needle) in [
        ("C=pidtune(tf(1,[1 1]),'PIX');", "type must be"),
        ("C=pidtune(tf(1,[1 1]),'PI',-1);", "positive"),
        ("C=pidtune(tf(1,[1 1],0.1),'PI');", "not supported yet"),
        ("C=pidtune(tf(1,[1 1]));", "requires"),
    ] {
        let message = error(&mut e, code);
        assert!(message.contains(needle), "{code}: {message}");
    }
}

/// Loop gain and phase (degrees) of C*G at w, from the engine's freqresp.
fn loop_at(e: &mut Engine, w: f64) -> (f64, f64) {
    e.execute(&format!(
        "h_=freqresp(C*G,{w:?}); mg_=abs(h_); ph_=angle(h_)*180/pi;"
    ))
    .unwrap();
    (number(e, "mg_"), number(e, "ph_"))
}

#[test]
fn pidtune_options_defaults_values_and_errors() {
    let mut e = Engine::default();
    e.execute("o=pidtuneOptions(); pm=o.PhaseMargin; df=o.DesignFocus; cl=class(o);")
        .unwrap();
    assert_eq!(number(&e, "pm"), 60.);
    assert_eq!(text(&e, "df"), "balanced");
    assert_eq!(text(&e, "cl"), "struct");
    e.execute(
        "o=pidtuneOptions('phasemargin',45,'DesignFocus','Reference-Tracking'); \
         pm=o.PhaseMargin; df=o.DesignFocus;",
    )
    .unwrap();
    assert_eq!(number(&e, "pm"), 45.);
    assert_eq!(text(&e, "df"), "reference-tracking");
    for (code, needle) in [
        ("o=pidtuneOptions('PhaseMargin');", "name-value"),
        ("o=pidtuneOptions('PhaseMargin',0);", "between 0 and 180"),
        ("o=pidtuneOptions('PhaseMargin',180);", "between 0 and 180"),
        ("o=pidtuneOptions('PhaseMargin',[30 40]);", "real scalar"),
        (
            "o=pidtuneOptions('DesignFocus','fast');",
            "DesignFocus must be",
        ),
        ("o=pidtuneOptions('Bogus',1);", "unknown option 'Bogus'"),
        (
            "C=pidtune(tf(1,[1 1]),'PI',struct('x',1));",
            "pidtuneOptions struct",
        ),
        ("C=pidtune(tf(1,[1 1]),'PI',1,2);", "pidtuneOptions struct"),
        (
            "C=pidtune(tf(1,[1 1]),'PI',[],struct('PhaseMargin',-1));",
            "pidtune: PhaseMargin must be between",
        ),
        ("C=pidtune(tf(1,[1 1]),'PIDX');", "'PDF'"),
    ] {
        let message = error(&mut e, code);
        assert!(message.contains(needle), "{code}: {message}");
    }
}

#[test]
fn pidtune_phase_margin_option_moves_the_design_target() {
    let mut e = Engine::default();
    // Default PID crossover for 1/(s+1)^3 with a 45 degree target: the plant
    // phase is 45 - 180 - 30 = -165 degrees, 3*atan(w) = 165, w = tan(55).
    e.execute(
        "G=tf(1,[1 3 3 1]); [C,info]=pidtune(G,'PID',pidtuneOptions('PhaseMargin',45)); \
         wc=info.CrossoverFrequency; pm=info.PhaseMargin; ok=isstable(feedback(C*G,1));",
    )
    .unwrap();
    let wc = number(&e, "wc");
    assert!((wc - 55_f64.to_radians().tan()).abs() < 1e-9, "{wc}");
    assert!((number(&e, "pm") - 45.).abs() < 1e-6);
    assert_eq!(number(&e, "ok"), 1.);
    let (mg, ph) = loop_at(&mut e, wc);
    assert!(
        (mg - 1.).abs() < 1e-9 && (ph + 135.).abs() < 1e-6,
        "{mg} {ph}"
    );
    // A given wc with a target margin: every two-gain type meets both. At
    // 1.5 rad/s the plant lags 3*atan(1.5) = 168.7 degrees, so each target
    // needs a controller phase inside the type's range.
    for (kind, margin) in [("PD", 50.), ("PDF", 70.), ("PID", 40.), ("PIDF", 75.)] {
        e.execute(&format!(
            "G=tf(1,[1 3 3 1]); [C,info]=pidtune(G,'{kind}',1.5,pidtuneOptions('PhaseMargin',{margin})); \
             pm=info.PhaseMargin; wc=info.CrossoverFrequency;"
        ))
        .unwrap_or_else(|err| panic!("{kind}: {err}"));
        assert_eq!(number(&e, "wc"), 1.5);
        assert!((number(&e, "pm") - margin).abs() < 1e-6, "{kind}");
        let (mg, ph) = loop_at(&mut e, 1.5);
        assert!(
            (mg - 1.).abs() < 1e-9 && (ph - (margin - 180.)).abs() < 1e-6,
            "{kind}: {mg} {ph}"
        );
    }
    // PI with a given wc solves Kp - j*Ki/wc = exp(j*(pm-180))/G(j*wc).
    // G = 1/(s(s+2)) at s = 0.5j: 1/G = -0.25 + 1j.
    e.execute(
        "G=tf(1,[1 2 0]); [C,info]=pidtune(G,'PI',0.5,pidtuneOptions('PhaseMargin',50)); \
         kp=C.Kp; ki=C.Ki; pm=info.PhaseMargin;",
    )
    .unwrap();
    let phase = (-130_f64).to_radians();
    let (re, im) = (phase.cos(), phase.sin());
    let (target_re, target_im) = (re * -0.25 - im * 1., re * 1. + im * -0.25);
    assert!((number(&e, "kp") - target_re).abs() < 1e-12);
    assert!((number(&e, "ki") + 0.5 * target_im).abs() < 1e-12);
    assert!((number(&e, "pm") - 50.).abs() < 1e-9);
    // wc = [] keeps the default crossover; DesignFocus does not change gains.
    e.execute(
        "G=tf(1,[1 3 3 1]); a=pidtune(G,'PIDF'); \
         b=pidtune(G,'PIDF',[],pidtuneOptions('DesignFocus','disturbance-rejection')); \
         same=[a.Kp a.Ki a.Kd a.Tf]==[b.Kp b.Ki b.Kd b.Tf];",
    )
    .unwrap();
    assert!(real(&e, "same").2.iter().all(|v| *v == 1.));
}

#[test]
fn pidtune_i_and_pdf_types() {
    let mut e = Engine::default();
    // I: the default crossover is where the plant phase is 60 - 180 + 90 =
    // -30 degrees: 3*atan(w) = 30, w = tan(10); Ki = w*|1/G(jw)| = w*(1+w^2)^1.5.
    e.execute(
        "G=tf(1,[1 3 3 1]); [C,info]=pidtune(G,'I'); wc=info.CrossoverFrequency; \
         pm=info.PhaseMargin; kp=C.Kp; ki=C.Ki; kd=C.Kd; tfv=C.Tf;",
    )
    .unwrap();
    let w = 10_f64.to_radians().tan();
    assert!((number(&e, "wc") - w).abs() < 1e-9);
    let ki = w * (1. + w * w).powf(1.5);
    assert!((number(&e, "ki") - ki).abs() < 1e-9 * ki);
    assert_eq!(
        (number(&e, "kp"), number(&e, "kd"), number(&e, "tfv")),
        (0., 0., 0.)
    );
    assert!((number(&e, "pm") - 60.).abs() < 1e-6);
    // PDF: no integral, Tf = 1/(10 wc), 60 degrees at the default crossover
    // where 3*atan(w) = 150.
    e.execute(
        "G=tf(1,[1 3 3 1]); [C,info]=pidtune(G,'PDF'); wc=info.CrossoverFrequency; \
         pm=info.PhaseMargin; kp=C.Kp; ki=C.Ki; kd=C.Kd; tfv=C.Tf; ok=isstable(feedback(C*G,1));",
    )
    .unwrap();
    let wc = number(&e, "wc");
    assert!((wc - 50_f64.to_radians().tan()).abs() < 1e-9);
    assert!(number(&e, "kp") > 0. && number(&e, "kd") > 0.);
    assert_eq!(number(&e, "ki"), 0.);
    assert!((number(&e, "tfv") - 1. / (10. * wc)).abs() < 1e-15);
    assert!((number(&e, "pm") - 60.).abs() < 1e-6);
    assert_eq!(number(&e, "ok"), 1.);
    let (mg, ph) = loop_at(&mut e, wc);
    assert!(
        (mg - 1.).abs() < 1e-9 && (ph + 120.).abs() < 1e-6,
        "{mg} {ph}"
    );
}

#[test]
fn help_and_names_registered() {
    for name in [
        "lyap",
        "dlyap",
        "pzmap",
        "pid",
        "pidstd",
        "pidtune",
        "pidtuneOptions",
    ] {
        assert!(matrust::help::is_supported(name), "{name}");
        assert!(matrust::help::page(name).is_some(), "{name}");
    }
}
