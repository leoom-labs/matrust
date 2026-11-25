//! python-control/SciPy numerical facts; regenerate with tool/refs/control/lti.py.
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
    tolerance: Option<f64>,
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
        Array::Complex(m) => m.data().iter().map(|x| (x.re, x.im)).collect(),
        Array::Logical(m) => m.data().iter().map(|&x| (f64::from(x), 0.)).collect(),
        _ => panic!("Expected numerical array, got {}", a.class()),
    }
}
#[test]
fn python_control_fixtures() {
    let fixture: Fixture = serde_json::from_str(include_str!("fixtures/control/lti.json")).unwrap();
    assert!(fixture.origin.contains("python-control 0.10.2"));
    for case in fixture.cases {
        let tolerance = case.tolerance.unwrap_or(2e-9);
        let mut e = Engine::default();
        e.execute(&case.code)
            .unwrap_or_else(|err| panic!("{}: {err}\n{}", case.name, case.code));
        for (name, reference) in case.expected {
            let a = e
                .array(&name)
                .unwrap_or_else(|| panic!("{} missing {name}", case.name));
            assert_eq!(
                (a.rows(), a.cols()),
                (reference.rows, reference.cols),
                "{} {name}",
                case.name
            );
            let mut actual = parts(&a);
            let mut expected: Vec<_> = reference.real.into_iter().zip(reference.imag).collect();
            if matches!(name.as_str(), "p" | "pd" | "z") {
                let cmp =
                    |a: &(f64, f64), b: &(f64, f64)| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1));
                actual.sort_by(cmp);
                expected.sort_by(cmp);
            }
            for (i, (a, b)) in actual.iter().zip(&expected).enumerate() {
                assert!(
                    (a.0 - b.0).abs() < tolerance * (1. + b.0.abs())
                        && (a.1 - b.1).abs() < tolerance * (1. + b.1.abs()),
                    "{} {name}[{i}]: {a:?} != {b:?}",
                    case.name
                );
            }
        }
    }
}
fn number(e: &Engine, n: &str) -> f64 {
    parts(&e.array(n).unwrap())[0].0
}
#[test]
fn value_semantics_properties_and_cells() {
    let mut e = Engine::default();
    e.execute("G=tf(1,[1 2]); G2=G; G2.Ts=0.1; t=G.Ts; t2=G2.Ts; G2.Numerator={3}; n=G.Numerator{1}; n2=G2.Numerator{1}; C={G}; C{1}.Denominator{1}(2)=4; d=G.Denominator{1}; dc=C{1}.Denominator{1};").unwrap();
    assert_eq!(number(&e, "t"), 0.);
    assert_eq!(number(&e, "t2"), 0.1);
    assert_eq!(parts(&e.array("n").unwrap()), vec![(0., 0.), (1., 0.)]);
    assert_eq!(parts(&e.array("n2").unwrap()), vec![(0., 0.), (3., 0.)]);
    assert_eq!(parts(&e.array("d").unwrap()), vec![(1., 0.), (2., 0.)]);
    assert_eq!(parts(&e.array("dc").unwrap()), vec![(1., 0.), (4., 0.)]);
    e.execute("S=ss(-2,1,3,4); Q=S; Q.A=-5; a=S.A; Z=zpk(-1,-2,3); Z2=Z; Z2.K=4; k=Z.K; Z2.Z={-3}; z=Z.Z{1}; S.InputName='command'; S.OutputName={'position'}; G.Variable='s';").unwrap();
    assert_eq!(number(&e, "a"), -2.);
    assert_eq!(number(&e, "k"), 3.);
    assert_eq!(number(&e, "z"), -1.);
    assert!(e.execute("G.Denominator={0}").is_err());
    e.execute("d=G.Denominator{1};").unwrap();
    assert_eq!(parts(&e.array("d").unwrap()), vec![(1., 0.), (2., 0.)]);
}
#[test]
fn classes_promotion_and_display() {
    let mut e = Engine::default();
    e.execute("G=tf(1,[1 2]); Z=zpk(G); S=ss(G); a=isa(G,'tf'); b=isa(Z,'lti'); c=isa(S,'ss'); q=isa(G,'struct'); X=G+Z; y=isa(X,'zpk'); X=G+S; z=isa(X,'ss'); cl=class(G);").unwrap();
    for name in ["a", "b", "c", "y", "z"] {
        assert_eq!(number(&e, name), 1.);
    }
    assert_eq!(number(&e, "q"), 0.);
    let Array::Character(c) = e.array("cl").unwrap() else {
        panic!()
    };
    assert_eq!(String::from_utf16_lossy(c.units()), "tf");
    for (name, markers) in [
        (
            "G",
            vec!["Continuous-time transfer function.", "s + 2", "-----"],
        ),
        (
            "Z",
            vec!["Continuous-time zero/pole/gain model.", "(s+2)", "-----"],
        ),
        (
            "S",
            vec!["Continuous-time state-space model.", "A =", "D ="],
        ),
    ] {
        let output = e.execute(name).unwrap().output.join("\n");
        for marker in markers {
            assert!(output.contains(marker), "{marker}: {output}");
        }
        let output = e
            .execute(&format!("disp({name})"))
            .unwrap()
            .output
            .join("\n");
        assert!(!output.contains("MatRustLTI"));
    }
}
#[test]
fn bounds_domains_and_invalid_models() {
    let mut e = Engine::default();
    for code in [
        "tf(1,0)",
        "tf([1 NaN],1)",
        "tf(1,1,-2)",
        "tf('z',0)",
        "zpk(1i,[],1)",
        "ss([1 2],1,1,0)",
        "tf({1,2},{1,1;1,1})",
        "tf({1,2},{1,0})",
        "ss(tf([1 1],1))",
        "tf(1,[1 1])^0.5",
        "tf(1,[1 1])/0",
        "feedback(ss(-1,1,1,1),ss(-2,1,1,1),1)",
        "c2d(tf(1,[1 1]),0.1,'bad')",
        "d2c(tf(1,[1 1]))",
        "d2c(tf(1,[1 1],0.1),'foh')",
        "tf(1,[1 1])+tf(1,[1 1],0.1)",
        "minreal(tf(1,1),-1)",
    ] {
        assert!(e.execute(code).is_err(), "Accepted {code}");
    }
    e.execute("G=tf(1,[1 1]);").unwrap();
    for code in [
        "G.Ts=-2",
        "G.A=1",
        "G.Variable='q'",
        "G.MatRustLTI='ss'",
        "G.InputName={1}",
    ] {
        assert!(e.execute(code).is_err(), "Accepted {code}");
    }
    e.execute("n=order(G); t=G.Ts; k=dcgain(G);").unwrap();
    assert_eq!(number(&e, "n"), 1.);
    assert_eq!(number(&e, "t"), 0.);
    assert_eq!(number(&e, "k"), 1.);
}
#[test]
fn help_registered_for_every_control_name() {
    for name in [
        "tf", "zpk", "ss", "tfdata", "zpkdata", "ssdata", "feedback", "series", "parallel", "pole",
        "zero", "dcgain", "minreal", "c2d", "d2c", "isstable", "issiso", "order", "damp", "pade",
    ] {
        assert!(matrust::help::is_supported(name), "{name}");
        assert!(matrust::help::page(name).is_some(), "{name}");
    }
}

#[test]
fn generic_struct_functions_respect_lti_properties() {
    for (constructor, model_fields) in [
        ("tf(1,[1 1])", vec!["Numerator", "Denominator"]),
        ("zpk([],-1,1)", vec!["Z", "P", "K"]),
        ("ss(-1,1,1,0)", vec!["StateName", "A", "B", "C", "D"]),
    ] {
        let mut e = Engine::default();
        e.execute(&format!("G={constructor}; f=fieldnames(G); count=numfields(G); flags=isfield(G,{{'Ts','MatRustLTI';'missing','InputName'}}); private=isfield(G,'MatRustLTI'); t=getfield(G,'Ts');")).unwrap();
        let Array::Cell(fields) = e.array("f").unwrap() else {
            panic!()
        };
        let actual: Vec<_> = (0..fields.len())
            .map(|i| {
                let Array::Character(c) = fields.get_linear(i).unwrap().as_ref() else {
                    panic!()
                };
                String::from_utf16_lossy(c.units())
            })
            .collect();
        let expected: Vec<_> = [
            "Ts",
            "Variable",
            "InputName",
            "OutputName",
            "InputDelay",
            "OutputDelay",
            "IODelay",
            "Name",
            "Notes",
            "UserData",
        ]
        .into_iter()
        .chain(model_fields)
        .collect();
        assert_eq!(actual, expected);
        assert_eq!((fields.rows(), fields.cols()), (expected.len(), 1));
        assert_eq!(number(&e, "count"), expected.len() as f64);
        assert_eq!(
            parts(&e.array("flags").unwrap()),
            vec![(1., 0.), (0., 0.), (0., 0.), (1., 0.)]
        );
        assert_eq!(
            (
                e.array("flags").unwrap().rows(),
                e.array("flags").unwrap().cols()
            ),
            (2, 2)
        );
        assert_eq!(number(&e, "private"), 0.);
        assert_eq!(number(&e, "t"), 0.);
        for code in [
            "G=setfield(G,'Ts',0.1)",
            "G=setfield(G,'MatRustLTI','tf')",
            "G=rmfield(G,'Ts')",
            "G=rmfield(G,'MatRustLTI')",
            "struct2cell(G)",
            "getfield(G,'MatRustLTI')",
        ] {
            assert!(
                e.execute(code).is_err(),
                "Accepted {code} for {constructor}"
            );
            e.execute("t=G.Ts; k=dcgain(G); o=order(G);").unwrap();
            assert_eq!(number(&e, "t"), 0.);
            assert_eq!(number(&e, "k"), 1.);
            assert_eq!(number(&e, "o"), 1.);
        }
        e.execute("Q=G; Q.Ts=0.1; t=G.Ts; tq=Q.Ts; v=Q.Variable;")
            .unwrap();
        assert_eq!(number(&e, "t"), 0.);
        assert_eq!(number(&e, "tq"), 0.1);
        let Array::Character(v) = e.array("v").unwrap() else {
            panic!()
        };
        assert_eq!(String::from_utf16_lossy(v.units()), "z");
    }
}

#[test]
fn inverse_and_quotient_signal_names_follow_ports() {
    for kind in ["tf", "zpk", "ss"] {
        let mut e = Engine::default();
        e.execute(&format!("G={kind}(tf([1 2],[1 1])); G.InputName='u'; G.OutputName='y'; H=G; H.InputName='v'; H.OutputName='w'; R=inv(G); ri=R.InputName{{1}}; ro=R.OutputName{{1}}; Q=G/H; qi=Q.InputName{{1}}; qo=Q.OutputName{{1}};")).unwrap();
        for (name, expected) in [("ri", "y"), ("ro", "u"), ("qi", "w"), ("qo", "y")] {
            let Array::Character(c) = e.array(name).unwrap() else {
                panic!()
            };
            assert_eq!(
                String::from_utf16_lossy(c.units()),
                expected,
                "{kind} {name}"
            );
        }
    }
}

#[test]
fn state_matrices_share_storage_on_assignment_and_metadata_writes() {
    let mut e = Engine::default();
    e.execute("A=-eye(64); S=ss(A,ones(64,1),ones(1,64),0); Q=S; Q.Ts=0.1;")
        .unwrap();
    let a = e.array("A").unwrap();
    let Array::Numeric(a) = a else { panic!() };
    for name in ["S", "Q"] {
        let Array::Struct(s) = e.array(name).unwrap() else {
            panic!()
        };
        let Array::Numeric(m) = s.get(s.field_index("A").unwrap(), 0).unwrap().as_ref() else {
            panic!()
        };
        assert_eq!(a.data().as_ptr(), m.data().as_ptr(), "{name} copied A");
    }
    let output = e.execute("whos").unwrap().output.join("\n");
    assert!(
        output
            .lines()
            .any(|line| line.starts_with('S') && line.ends_with("ss")),
        "{output}"
    );
}
