//! MIMO LTI model behavior and python-control/SciPy reference cases.
use matrust::{Array, Engine};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Deserialize)]
struct Reference {
    origin: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    code: String,
    expected: BTreeMap<String, Expected>,
}

#[derive(Deserialize)]
struct Expected {
    rows: usize,
    cols: usize,
    real: Vec<f64>,
    imag: Vec<f64>,
}

#[test]
fn control_mimo_python_control_fixtures() {
    let reference: Reference =
        serde_json::from_str(include_str!("fixtures/control/lti_mimo.json")).unwrap();
    assert!(reference.origin.contains("python-control 0.10.2; SciPy"));
    for case in reference.cases {
        let mut e = Engine::default();
        e.execute(&case.code)
            .unwrap_or_else(|error| panic!("{}: {error}\n{}", case.name, case.code));
        for (name, expected) in case.expected {
            let value = e.array(&name).unwrap();
            assert_eq!(
                (value.rows(), value.cols()),
                (expected.rows, expected.cols),
                "{} {name}",
                case.name
            );
            let data: Vec<_> = match value {
                Array::Numeric(m) => m.data().iter().map(|&v| (v, 0.)).collect(),
                Array::Logical(m) => m.data().iter().map(|&v| (f64::from(v), 0.)).collect(),
                Array::Complex(m) => m.data().iter().map(|v| (v.re, v.im)).collect(),
                _ => panic!("{} {name}: expected numerical data", case.name),
            };
            assert_eq!(data.len(), expected.real.len());
            for (i, ((re, im), (er, ei))) in data
                .into_iter()
                .zip(expected.real.into_iter().zip(expected.imag))
                .enumerate()
            {
                assert!(
                    (re - er).abs() <= 2e-9 * (1. + er.abs())
                        && (im - ei).abs() <= 2e-9 * (1. + ei.abs()),
                    "{} {name}[{i}]: ({re},{im}) != ({er},{ei})",
                    case.name
                );
            }
        }
    }
}

#[test]
fn control_mimo_shapes_cells_and_properties() {
    let mut e = Engine::default();
    e.execute(concat!(
        "F=tf({1,[2 3];4,5},{[2 1],[1 4 5];[2 6],1},0.25); ",
        "assert(isequal(size(F),[2 2])); assert(size(F,1)==2); assert(size(F,2)==2); ",
        "assert(size(F,3)==1); assert(size(F,7)==1); assert(~isempty(F)); assert(~issiso(F)); ",
        "assert(order(F)==4); [n,d,t]=tfdata(F); ",
        "assert(isequal(size(n),[2 2])); assert(isequal(size(d),[2 2])); assert(t==0.25); ",
        "assert(isequal(n,F.Numerator)); assert(isequal(d,F.Denominator)); ",
        "assert(isequal(n{1,2},[0 2 3])); assert(isequal(d{2,1},[2 6])); ",
        "F.Denominator{2,1}=[3 4]; assert(isequal(F.Denominator{2,1},[3 4])); ",
        "Z=zpk({-2,[];[],[]},{[-1 -3],-2;[-1+2i;-1-2i],[]},[3 -4;5 2]); ",
        "assert(isequal(size(Z),[2 2])); assert(order(Z)==5); ",
        "[z,p,k,t]=zpkdata(Z); assert(isequal(size(z),[2 2])); ",
        "assert(isequal(size(p),[2 2])); assert(isequal(k,[3 -4;5 2])); assert(t==0); ",
        "assert(isequal(p{1,1},[-1;-3])); assert(isequal(p{2,1},[-1+2i;-1-2i])); ",
        "Z.K(2,1)=7; assert(Z.K(2,1)==7); ",
        "X=tf({1,2},{[1 1],[1 2]}); [ny,nu]=size(X); assert(ny==1 && nu==2); ",
        "J=tf({1,2},{[1 -0.5],[1 -0.25]},0.2,'Variable','z^-1'); ",
        "before=evalfr(J,2i); J.Variable='q'; after=evalfr(J,2i); ",
        "assert(norm(before-after)<1e-12); J.Variable='q^-1'; ",
        "assert(norm(before-evalfr(J,2i))<1e-12); ",
    ))
    .unwrap();
}

#[test]
fn control_mimo_empty_ports_remain_shaped() {
    let mut e = Engine::default();
    e.execute(concat!(
        "assert(isempty(tf())); assert(isempty(zpk())); assert(isempty(ss())); ",
        "S=ss(1,2,[],[]); assert(isempty(S)); assert(isequal(size(S),[0 1])); ",
        "assert(order(S)==1); assert(~issiso(S)); ",
        "F=tf({1,2},{[1 1],[1 2]}); E=F([],:); ",
        "assert(isempty(E)); assert(isequal(size(E),[0 2])); assert(order(E)==0); ",
        "assert(isequal(size(tfdata(E)),[0 2])); assert(isequal(size(ss(E)),[0 2])); ",
        "Z=zpk(F); Q=Z(:,[]); assert(isequal(size(tf(Q)),[1 0])); ",
        "T=ss(-1,[1 2],[1;2],0); U=T(:,[]); ",
        "assert(isempty(U)); assert(isequal(size(U),[2 0])); assert(order(U)==1); ",
        "assert(isequal(size(U.B),[1 0])); assert(isequal(size(U.D),[2 0])); ",
        "P=tf(1,[1 3]); assert(isequal(size(P([1 1],:)),[2 1])); ",
    ))
    .unwrap();
}

#[test]
fn control_mimo_selection_preserves_names_and_assignment_shares_cells() {
    let mut e = Engine::default();
    e.execute(concat!(
        "S=ss([0 1;-2 -3],[1 2 3;4 5 6],eye(2),zeros(2,3),",
        "'InputName',{'one','two','three'},'OutputName',{'alpha','beta'},'StateName',{'pos','vel'}); ",
        "F=tf(S); Z=zpk(F); ",
    )).unwrap();
    let checks = concat!(
        "R=M(2,:); C=M(:,end); Q=M([2 1 2],[3 1]); ",
        "assert(isequal(size(R),[1 3])); assert(isequal(R.OutputName,{'beta'})); ",
        "assert(isequal(size(C),[2 1])); assert(isequal(C.InputName,{'three'})); ",
        "assert(isequal(Q.OutputName,{'beta';'alpha';'beta'})); ",
        "assert(isequal(Q.InputName,{'three';'one'})); assert(issiso(M(end,2))); ",
    );
    for model in ["S", "F", "Z"] {
        e.execute(&format!("M={model}; {checks}"))
            .unwrap_or_else(|error| panic!("{model}: {error}"));
    }
    e.execute(concat!(
        "P=S(2,1); assert(isequal(P.StateName,{'pos';'vel'})); assert(isequal(P.A,S.A)); ",
        "A=F; rhs=tf(9,[1 7]); F(2,end)=rhs; ",
        "keep=A.Numerator; old=F.Numerator; ",
        "box={Z}; box{1}(2,3)=rhs; Z2=box{1}; ",
        "assert(isequal(Z2.InputName,Z.InputName)); assert(isequal(Z2.OutputName,Z.OutputName)); ",
        "assert(abs(evalfr(Z2(2,3),1)-9/8)<1e-12); ",
    ))
    .unwrap();
    let slots = |name| match e.array(name).unwrap() {
        Array::Cell(c) => c,
        _ => panic!("expected coefficient cells"),
    };
    let (old, current) = (slots("keep"), slots("old"));
    for i in 0..old.len() {
        assert_eq!(
            Arc::ptr_eq(old.get_linear(i).unwrap(), current.get_linear(i).unwrap()),
            i != 5
        );
    }
}

#[test]
fn control_mimo_reduction_preserves_model_class_and_reduces_order() {
    let mut e = Engine::default();
    e.execute(concat!(
        "F=tf({[1 1],2;0,[1 4]},{[1 3 2],[1 3];[1 9],[1 8 16]}); ",
        "G=minreal(F); H=minreal(zpk(F)); assert(isa(G,'tf')); assert(isa(H,'zpk')); ",
        "assert(order(F)==6); assert(order(G)==3); assert(order(H)==3); ",
        "A=[-1 -1 1;0 -2 -1;0 0 -3]; B=[3 3;2 1;0 0]; C=[3 -3 5;4 -4 5]; ",
        "S=ss(A,B,C,[0 2;4 0],0.2,'InputName',{'one','two'}); T=minreal(S); ",
        "assert(isa(T,'ss')); assert(order(T)==1); assert(T.Ts==0.2); ",
        "assert(isequal(T.InputName,S.InputName)); ",
    ))
    .unwrap();
}

#[test]
fn control_mimo_display_has_one_section_per_channel() {
    let mut e = Engine::default();
    e.execute("F=tf({1,2;3,4},{[1 1],[1 2];[1 3],[1 4]}); Z=zpk(F);")
        .unwrap();
    for (name, footer, factor) in [
        ("F", "Continuous-time transfer function.", "s + 4"),
        ("Z", "Continuous-time zero/pole/gain model.", "(s+4)"),
    ] {
        let body = e
            .execute(&format!("disp({name})"))
            .unwrap()
            .output
            .join("\n");
        let labels = [
            "  From input 1 to output 1:",
            "  From input 1 to output 2:",
            "  From input 2 to output 1:",
            "  From input 2 to output 2:",
        ];
        let positions: Vec<_> = labels.iter().map(|s| body.find(s).unwrap()).collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{body}");
        assert_eq!(body.matches("  From input").count(), 4, "{body}");
        assert_eq!(body.matches(footer).count(), 1, "{body}");
        assert!(body[positions[3]..].contains(factor), "{body}");
    }
    let named = e
        .execute("F.InputName={'command',''}; F.OutputName={'displacement','velocity'}; disp(F);")
        .unwrap()
        .output
        .join("\n");
    assert!(
        named.contains("  From input \"command\" to output \"displacement\":"),
        "{named}"
    );
    assert!(
        named.contains("  From input 2 to output \"velocity\":"),
        "{named}"
    );

    let ss = e
        .execute("disp(ss([0 1;-2 -3],[1 2 3;4 5 6],eye(2),[11 12 13;21 22 23]));")
        .unwrap()
        .output
        .join("\n");
    for (block, columns, rows) in [
        ("B", &["u1", "u2", "u3"][..], &["x1", "x2"][..]),
        ("C", &["x1", "x2"][..], &["y1", "y2"][..]),
        ("D", &["u1", "u2", "u3"][..], &["y1", "y2"][..]),
    ] {
        let table = ss
            .split_once(&format!("{block} ="))
            .unwrap()
            .1
            .split("\n\n")
            .next()
            .unwrap();
        let lines: Vec<_> = table.lines().filter(|s| !s.trim().is_empty()).collect();
        assert_eq!(lines[0].split_whitespace().collect::<Vec<_>>(), columns);
        for (line, row) in lines[1..].iter().zip(rows) {
            assert_eq!(line.split_whitespace().next().unwrap(), *row);
        }
    }
}

#[test]
fn control_mimo_diagnostics_and_atomic_channel_assignment() {
    let mut e = Engine::default();
    e.execute("F=tf({1,2},{[1 1],[1 2]}); S=ss(-1,[1 2],[1;2],0); Z=zpk(F);")
        .unwrap();
    for (expression, operation) in [
        ("zero(S)", "zero"),
        ("inv(F)", "inv"),
        ("tfdata(F,'v')", "tfdata"),
        ("zpkdata(Z,'v')", "zpkdata"),
        ("rlocus(F)", "rlocus"),
        ("pidtune(F,'PI')", "pidtune"),
    ] {
        let error = e.execute(expression).unwrap_err();
        assert!(
            error.contains(operation) && error.contains("SISO"),
            "{expression}: {error}"
        );
    }
    for expression in [
        "F(1,3)",
        "F(1,1,1)",
        "F(1,1)=Z",
        "F(:,1:2)=F(1,1)",
        "F(1,2)=[]",
        "F(1,2)=tf(1,[1 1],0.25)",
        "F(1,2)=tf(1,[1 1],'InputDelay',1)",
        "S(1,1)=tf(2)",
        "F.Denominator{1,2}=0",
        "F.Denominator={1;2}",
        "Z.K=[1;2]",
        "tf({1,2},{1;1})",
        "tf({1,2},{1,0})",
        "zpk({[],[]},{[],[]},[1;2])",
        "zpk({[],1i},{[],[]},[1 2])",
        "ss(tf({[1 0],2},{1,1}))",
    ] {
        assert!(e.execute(expression).is_err(), "{expression}");
    }
    e.execute("assert(abs(evalfr(F(1,2),1)-2/3)<1e-12); assert(Z.K(1,2)==2);")
        .unwrap();
}
