//! Independent SciPy/python-control/NumPy facts; regenerate with
//! bash tool/ref-python.sh tool/refs/control/design.py.
use matrust::{Array, Engine};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Fixture {
    origin: String,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    code: String,
    expected: BTreeMap<String, Expected>,
    tolerance: f64,
    unordered: Vec<String>,
}
#[derive(Deserialize)]
struct Expected {
    rows: usize,
    cols: usize,
    real: Vec<f64>,
    imag: Vec<f64>,
}
fn parts(a: &Array) -> Vec<(f64, f64)> {
    match a {
        Array::Numeric(m) => m.data().iter().map(|&x| (x, 0.)).collect(),
        Array::Complex(m) => m.data().iter().map(|z| (z.re, z.im)).collect(),
        _ => panic!("Expected numerical array"),
    }
}
fn number(e: &Engine, name: &str) -> f64 {
    parts(&e.array(name).unwrap())[0].0
}
fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}
#[test]
fn independent_design_fixtures() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/control/design.json")).unwrap();
    assert!(fixture.origin.contains("python-control 0.10.2"));
    for c in fixture.cases {
        let mut e = Engine::default();
        e.execute(&c.code)
            .unwrap_or_else(|err| panic!("{}: {err}\n{}", c.name, c.code));
        for (name, r) in c.expected {
            let a = e.array(&name).unwrap();
            assert_eq!((a.rows(), a.cols()), (r.rows, r.cols), "{} {name}", c.name);
            let actual = parts(&a);
            let expected: Vec<_> = r.real.into_iter().zip(r.imag).collect();
            if c.unordered.contains(&name) {
                // Eigenvalues as sets; for rlocus compare each gain column.
                for col in 0..r.cols {
                    let mut remaining = actual[col * r.rows..(col + 1) * r.rows].to_vec();
                    for &z in &expected[col * r.rows..(col + 1) * r.rows] {
                        let i = remaining
                            .iter()
                            .enumerate()
                            .min_by(|(_, a), (_, b)| distance(**a, z).total_cmp(&distance(**b, z)))
                            .unwrap()
                            .0;
                        let found = remaining.remove(i);
                        assert!(
                            distance(found, z) <= c.tolerance * (1. + z.0.hypot(z.1)),
                            "{} {name}: {found:?} != {z:?}, tolerance {}",
                            c.name,
                            c.tolerance
                        );
                    }
                }
            } else {
                for (&a, &b) in actual.iter().zip(&expected) {
                    assert!(
                        distance(a, b) <= c.tolerance * (1. + b.0.hypot(b.1)),
                        "{} {name}: {a:?} != {b:?}, tolerance {}",
                        c.name,
                        c.tolerance
                    );
                }
            }
        }
    }
}
#[test]
fn riccati_relative_residuals_and_output_order() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/control/design.json")).unwrap();
    for c in fixture
        .cases
        .into_iter()
        .filter(|c| c.name.starts_with("care_") || c.name.starts_with("dare_"))
    {
        let mut e = Engine::default();
        e.execute(&c.code).unwrap();
        let residual = if c.name.starts_with("dare") {
            "H=A'*X*A; F=(A'*X*B+N)*K; residual=norm(H-X-F+Q,'fro')/(norm(H,'fro')+norm(X,'fro')+norm(F,'fro')+norm(Q,'fro'));"
        } else {
            "H=A'*X+X*A; F=(X*B+N)*K; residual=norm(H-F+Q,'fro')/(norm(H,'fro')+norm(F,'fro')+norm(Q,'fro'));"
        };
        e.execute(residual).unwrap();
        assert!(
            number(&e, "residual") < 1e-10,
            "{} residual {}",
            c.name,
            number(&e, "residual")
        );
    }
    let mut e = Engine::default();
    e.execute("[X,K,L]=care(0,1,1); [Xd,Kd,Ld]=dare(1,1,1); [Kc,Sc,Pc]=lqr(ss(0,1,1,0),1,1); [Kd2,Sd2,Pd2]=lqr(ss(1,1,1,0,0.1),1,1); [Xe,Ke,Le]=care(0,1,1,[],[],1); [Xde,Kde,Lde]=dare(1,1,1,[],[],1);").unwrap();
    assert!((number(&e, "X") - 1.).abs() < 1e-12);
    assert!((number(&e, "K") - 1.).abs() < 1e-12);
    assert!((number(&e, "L") + 1.).abs() < 1e-12);
    for (a, b) in [
        ("K", "Kc"),
        ("X", "Sc"),
        ("L", "Pc"),
        ("Kd", "Kd2"),
        ("Xd", "Sd2"),
        ("Ld", "Pd2"),
        ("X", "Xe"),
        ("Xd", "Xde"),
    ] {
        assert!((number(&e, a) - number(&e, b)).abs() < 1e-12, "{a} {b}");
    }
}
#[test]
fn locus_branch_continuity_and_plot() {
    let mut e = Engine::default();
    e.execute("[R,k]=rlocus(tf(1,[1 3 2]));").unwrap();
    let k = e.array("k").unwrap();
    let r = e.array("R").unwrap();
    assert!((83..=4096).contains(&k.cols())); // at least one adaptive split
    assert_eq!((r.rows(), r.cols()), (2, k.cols()));
    let z = parts(&r);
    for j in 1..r.cols() {
        assert!(
            distance(z[2 * j], z[2 * (j - 1)]) <= distance(z[2 * j + 1], z[2 * (j - 1)]) + 1e-12
        );
    }
    e.execute("rlocus(tf([1 2],[1 4 5 0]),[0 .1 1 10 100]);")
        .unwrap();
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(figure.title, "Root locus");
    assert_eq!(figure.axes.series.len(), 5); // three branches plus pole/zero markers
}
#[test]
fn explicit_errors_and_bounds() {
    let mut e = Engine::default();
    for (code, part) in [
        ("place(eye(2),[1;0],[-1 -2])", "uncontrollable"),
        ("place([0 1;0 0],[0;1],[-1 -1])", "multiplicity"),
        ("place(eye(2),eye(2),[1i 2i])", "conjugate"),
        ("acker(eye(2),eye(2),[-1 -2])", "single input"),
        ("care(1,0,1)", "stabilis"),
        ("dare(2,0,1)", "stabilis"),
        ("care(0,1,1,1,0,2)", "descriptor E"),
        ("dare(1,1,1,1,0,2)", "descriptor E"),
        ("care(0,1,1,-1)", "positive definite"),
        ("care(eye(2),ones(2,1),[1 2;0 1])", "symmetric"),
        ("ctrb(eye(129),ones(129,1))", "128 states"),
        ("obsv(eye(2),ones(2,1))", "C must have"),
        ("rlocus(tf(1,[1 1]),[-1 0])", "nonnegative"),
        ("rlocus(tf([1 1],[1 2]),[0 1e309])", "NaN/Inf"),
        ("place([0 1;0 0],[0;1],[NaN -1])", "finite"),
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(err.contains(part), "{code}: {err}, expected {part}");
    }
}
#[test]
fn function_handles_and_help_registration() {
    let mut e = Engine::default();
    e.execute(
        "f=@care; [X,K,L]=f(0,1,1); [X2,K2,L2]=feval('care',0,1,1); c=exist('rlocus','builtin');",
    )
    .unwrap();
    assert_eq!(number(&e, "c"), 5.);
    for name in [
        "ctrb", "obsv", "care", "dare", "lqr", "dlqr", "place", "acker", "rlocus",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&name));
        let page = matrust::help::page(name).unwrap();
        assert!(page.contains("Differences from MATLAB:"));
    }
}

#[test]
fn pole_assignment_expands_rank_deficient_inputs_without_controllability_powers() {
    let mut e = Engine::default();
    // rank(B)=2 forces reachability expansion. ctrb loses the small modes
    // beside A^3*B=1e18, while orthogonal expansion recovers all four states.
    e.execute("A=diag([1e6 -1 -2 -3]); B=[1 0;0 1;0 1;0 1]; rb=rank(B); rc=rank(ctrb(A,B)); K=place(A,B,[-2 -3 -4 -5]); p=eig(A-B*K);")
        .unwrap();
    assert_eq!(number(&e, "rb"), 2.);
    assert!(number(&e, "rc") < 4.);
    let mut p = parts(&e.array("p").unwrap());
    p.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (&a, b) in p.iter().zip([-5., -4., -3., -2.]) {
        assert!((a.0 - b).abs() < 1e-8 && a.1.abs() < 1e-8);
    }
    // Removing the second input makes three modes unreachable.
    let err = e.execute("K=place(A,B(:,1),[-2 -3 -4 -5]);").unwrap_err();
    assert!(err.contains("uncontrollable"), "{err}");
}

#[test]
fn single_input_place_handles_integrator_chains_and_redundant_inputs() {
    // Requested poles are exact mathematical facts; pole errors are absolute.
    // The 12-state companion matrix is more sensitive, hence its looser bound.
    for (n, tolerance) in [(10, 1e-8), (12, 1e-6)] {
        for redundant in [false, true] {
            let mut e = Engine::default();
            let inputs = if redundant { "[b 2*b]" } else { "b" };
            e.execute(&format!("A=diag(ones(1,{}),1); b=zeros({n},1); b({n})=1; B={inputs}; K=place(A,B,-(1:{n})); p=eig(A-B*K);", n-1)).unwrap();
            let mut p = parts(&e.array("p").unwrap());
            p.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (i, z) in p.into_iter().enumerate() {
                assert!(
                    distance(z, (-((i + 1) as f64), 0.)) < tolerance,
                    "n={n}, redundant={redundant}, pole={z:?}, tolerance={tolerance}"
                );
            }
        }
    }
}

#[test]
fn single_input_pole_assignment_accepts_conjugates() {
    let mut e = Engine::default();
    e.execute("A=[0 1;0 0]; B=[0;1]; K=place(A,B,[-1+2i -1-2i]); p=eig(A-B*K);")
        .unwrap();
    for z in parts(&e.array("p").unwrap()) {
        assert!((z.0 + 1.).abs() < 1e-12 && (z.1.abs() - 2.).abs() < 1e-12);
    }
    e.execute("A=[0 1;0 0]; B=[0;1]; K=acker(A,B,[-1+2i -1-2i]); p=eig(A-B*K); Kr=acker(A,B,[-2 -2]); pr=eig(A-B*Kr);")
        .unwrap();
    for z in parts(&e.array("p").unwrap()) {
        assert!((z.0 + 1.).abs() < 1e-12 && (z.1.abs() - 2.).abs() < 1e-12);
    }
    for z in parts(&e.array("pr").unwrap()) {
        assert!((z.0 + 2.).abs() < 1e-10 && z.1.abs() < 1e-10);
    }
}

#[test]
fn sensitive_poles_warn_and_return_the_gain() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/control/design.json")).unwrap();
    let case = fixture
        .cases
        .into_iter()
        .find(|c| c.name == "place_sensitive_chain16")
        .unwrap();
    let mut e = Engine::default();
    let output = e.execute(&case.code).unwrap();
    assert!(
        output
            .output
            .iter()
            .any(|line| line.contains("closed-loop pole error exceeds"))
    );
    let gain = parts(&e.array("K").unwrap());
    for (actual, expected) in gain.iter().zip(&case.expected["K"].real) {
        assert!((actual.0 - expected).abs() < 1e-12 * expected.abs().max(1.));
    }
}

#[test]
fn locus_single_gain_plots_separate_root_markers() {
    use matrust::plot::{MarkerShape, PlotKind};
    for (num, expected_series) in [("1", 3), ("[1 4]", 4)] {
        let mut e = Engine::default();
        e.execute(&format!(
            "[R,k]=rlocus(tf({num},[1 3 2]),5); rlocus(tf({num},[1 3 2]),5);"
        ))
        .unwrap();
        let roots = parts(&e.array("R").unwrap());
        let figure = e.figure.as_ref().unwrap();
        let series = &figure.axes.series;
        assert_eq!(series.len(), expected_series); // two branches plus pole/zero sets
        for (branch, root) in series[..2].iter().zip(roots) {
            assert_eq!(branch.points.len(), 1);
            let point = branch.points.iter().next().unwrap();
            assert!(distance((point[0], point[1]), root) < 1e-12);
            assert_eq!(branch.effective_marker(), MarkerShape::Point);
            assert!(branch.markers);
            assert_eq!(branch.kind, PlotKind::Scatter);
        }
        assert_eq!(series[2].marker_shape, Some(MarkerShape::Cross));
        assert_eq!(series[2].points.len(), 2);
        if expected_series == 4 {
            assert_eq!(series[3].effective_marker(), MarkerShape::Circle);
            assert_eq!(series[3].points.len(), 1);
        }
    }
}

#[test]
fn riccati_seeded_sixty_state_solutions_match_scipy() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/control/design.json")).unwrap();
    let cases: Vec<_> = fixture
        .cases
        .into_iter()
        .filter(|c| c.name.ends_with("_seed4_n60"))
        .collect();
    assert_eq!(cases.len(), 2);
    for c in cases {
        let mut e = Engine::default();
        e.execute(&c.code)
            .unwrap_or_else(|err| panic!("{}: {err}", c.name));
        for name in ["X", "K"] {
            let actual = parts(&e.array(name).unwrap());
            let expected = &c.expected[name].real;
            let error = actual
                .iter()
                .zip(expected)
                .fold(0_f64, |norm, (a, b)| norm.hypot(a.0 - b));
            let scale = expected.iter().fold(0_f64, |norm, x| norm.hypot(*x));
            assert!(
                error / scale < 2e-8,
                "{} {name}: relative error {}",
                c.name,
                error / scale
            );
        }
        for z in parts(&e.array("L").unwrap()) {
            assert!(if c.name.starts_with("care") {
                z.0 < 0.
            } else {
                z.0.hypot(z.1) < 1.
            });
        }
    }
}
