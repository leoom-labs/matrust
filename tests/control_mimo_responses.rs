//! Channel order, numerical responses and interconnections from python-control and SciPy.
use matrust::{Array, Engine};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Reference {
    origin: String,
    response_cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    code: String,
    expected: BTreeMap<String, Expected>,
}

#[derive(Deserialize)]
struct Expected {
    dims: Vec<usize>,
    real: Vec<f64>,
    imag: Vec<f64>,
}

#[test]
fn control_mimo_response_and_interconnection_fixtures() {
    let reference: Reference =
        serde_json::from_str(include_str!("fixtures/control/lti_mimo.json")).unwrap();
    assert!(reference.origin.contains("python-control 0.10.2; SciPy"));
    for case in reference.response_cases {
        let mut e = Engine::default();
        e.execute(&case.code)
            .unwrap_or_else(|error| panic!("{}: {error}\n{}", case.name, case.code));
        for (name, expected) in case.expected {
            let array = e.array(&name).unwrap();
            assert_eq!(array.dims().as_ref(), expected.dims, "{} {name}", case.name);
            let values: Vec<_> = match array {
                Array::Numeric(m) => m.data().iter().map(|&r| (r, 0.)).collect(),
                Array::Complex(m) => m.data().iter().map(|v| (v.re, v.im)).collect(),
                _ => panic!("{} {name}: expected a double array", case.name),
            };
            assert_eq!(values.len(), expected.real.len());
            for (i, ((re, im), (want_re, want_im))) in values
                .into_iter()
                .zip(expected.real.into_iter().zip(expected.imag))
                .enumerate()
            {
                assert!(
                    (re - want_re).abs() <= 2e-8 * (1. + want_re.abs())
                        && (im - want_im).abs() <= 2e-8 * (1. + want_im.abs()),
                    "{} {name}[{i}]: ({re},{im}) != ({want_re},{want_im})",
                    case.name
                );
            }
        }
    }
}

#[test]
fn control_mimo_response_shapes_and_model_precedence() {
    let mut e = Engine::default();
    e.execute(concat!(
        "G=ss([0 1;-2 -3],eye(2),eye(2),zeros(2)); F=tf(G); Z=zpk(F); t=0:.5:1;",
        "y=step(G,t); a=step(F,t); b=step(Z,t); assert(max(abs(y(:)-a(:)))<1e-9);",
        "assert(max(abs(y(:)-b(:)))<1e-9); assert(abs(y(2,2,1)+.15481812174617549)<1e-9);",
        "S=G(1,1); [y,tout,x]=step(S,t); assert(isequal(size(y),[3 1]));",
        "assert(isequal(size(x),[3 2])); assert(isequal(size(tout),[3 1]));",
        "[m,p,w]=bode(S,[.2 1 2]); H=freqresp(S,[.2 1 2]);",
        "assert(isequal(size(m),[3 1]) && isequal(size(H),[3 1]));",
        "assert(strcmp(class(F+F),'tf')); assert(strcmp(class(F+Z),'zpk'));",
        "assert(strcmp(class(F+G),'ss')); assert(strcmp(class(feedback(F,eye(2))),'tf'));",
        "assert(strcmp(class(feedback(Z,F)),'zpk')); assert(strcmp(class(append(F,Z,G)),'ss'));",
        "assert(strcmp(class([F Z]),'zpk')); assert(strcmp(class([F;F]),'tf'));",
        "assert(isequal(size([G,G;G,zeros(2)]),[4 4]));",
        "assert(isequal(size(ss(eye(2))),[2 2])); assert(isequal(size(tf(eye(2))),[2 2]));",
        "E=append(G(:,[]),G([],:)); assert(isequal(size(E),[2 2]) && order(E)==4);",
        "G.InputName={'a';'b'}; G.OutputName={'c';'d'}; G.StateName={'x';'v'};",
        "R=feedback(G,1,2,1); assert(isequal(R.StateName,G.StateName));",
        "assert(isequal(R.InputName,G.InputName) && isequal(R.OutputName,G.OutputName));",
        "H=G; H.InputName={'b';'f'}; H.OutputName={'g';'c'};",
        "S=parallel(G,H,2,1,1,2); assert(isequal(S.InputName,{'a';'b';'f'}));",
        "assert(isequal(S.OutputName,{'d';'c';'g'}));",
        "H.InputName={'e';'f'}; S=parallel(G,H,2,1,1,2); assert(isequal(S.InputName,{'a';'';'f'}));",
        "S=ss(-1,zeros(1,0),[1;2],zeros(2,0)); [y,t,x]=initial(S,1,0:.5:1);",
        "assert(isequal(size(y),[3 2]) && abs(x(2)-exp(-.5))<1e-12);",
        "V=tf({[1 0],1},{1,[1 1]}); h=freqresp(V,[.2 1]); assert(abs(h(1,1,2)-1i)<1e-12);",
    ))
    .unwrap();
}

#[test]
fn control_mimo_rational_automatic_frequency_grid() {
    let mut e = Engine::default();
    // Documented pole-only grid for poles -1 and -2 (Bode and Nyquist help).
    e.execute(concat!(
        "G=tf({1,2},{[1 1],[1 2]}); [m,p,w]=bode(G);",
        "[r,i,v]=nyquist(G); [~,~,s]=bode(ss(G)); ",
        "[mz,~,z]=bode(zpk(G)); ",
        "assert(isequal(size(m),[1 2 1000]) && isequal(size(p),size(m)));",
        "assert(isequal(size(r),size(m)) && isequal(size(i),size(m)));",
        "assert(isequal(size(mz),size(m)) && isequal(w,v,s,z));",
        "assert(abs(w(1)-.1)<1e-12 && abs(w(end)-100)<1e-12);",
        "assert(abs(m(1,2,1)-2/sqrt(4.01))<1e-12 && abs(r(1,2,1)-4/4.01)<1e-12);",
    ))
    .unwrap();
}

#[test]
fn control_delayed_parallel_phase_keeps_complete_turns_on_sparse_grid() {
    let mut e = Engine::default();
    // NumPy unwrap(angle(1 + 2*exp(-1j*w))) on a dense grid gives -360°
    // at 2*pi. The magnitude and frequency vectors keep the requested samples.
    e.execute(concat!(
        "G=tf(1)+tf(2,1,'InputDelay',1); [m,p,w]=bode(G,[0 2*pi]);",
        "assert(isequal(w,[0;2*pi]) && max(abs(m-[3;3]))<1e-12);",
        "assert(max(abs(p-[0;-360]))<1e-10);",
        "[~,q]=nichols(G,[0 2*pi 4*pi]); assert(max(abs(q-[0;-360;-720]))<1e-10);",
        "h=freqresp(G,[0 10000]); assert(numel(h)==2);",
        "m=bode(G,[0 10000]); assert(max(abs(m-abs(h)))<1e-12);",
    ))
    .unwrap();
    let error = e.execute("[m,p]=bode(G,[0 10000]);").unwrap_err();
    assert!(
        error.contains("Delayed response phase needs more than 10000 points"),
        "{error}"
    );
}

#[test]
fn control_mimo_rational_stepinfo_and_allmargin() {
    let mut e = Engine::default();
    e.execute(concat!(
        "G=tf({1,2},[1 1]); S=stepinfo(G,[0 1]); Z=stepinfo(zpk(G),[0 1]);",
        "assert(isequal(size(S),[1 2]) && isequal(size(Z),size(S)));",
        "assert(abs(S(1).Peak-(1-exp(-1)))<1e-12);",
        "assert(abs(S(2).Peak-2*(1-exp(-1)))<1e-12 && Z(2).Peak==S(2).Peak);",
        "G=tf(eye(2)); M=allmargin(G); N=allmargin(zpk(G));",
        "assert(isequal(size(M),[2 1]) && isequal(size(N),size(M)));",
        "assert(M(1).Stable==1 && M(2).Stable==1 && N(1).Stable==1 && N(2).Stable==1);",
    ))
    .unwrap();
    let error = e.execute("M=allmargin(tf({1,2},[1 1]));").unwrap_err();
    assert!(
        error.contains("allmargin requires a SISO or square MIMO loop transfer"),
        "{error}"
    );
}

#[test]
fn control_mimo_interconnection_diagnostics() {
    let mut e = Engine::default();
    e.execute("A=ss(eye(2)); B=ss([1 2 3;4 5 6]);").unwrap();
    for (code, text) in [
        ("series(A,B)", "number of outputs of the first model"),
        ("B*A", "Model I/O dimensions must agree."),
        ("A+B", "same number of inputs and outputs"),
        ("parallel(A,B)", "same number of inputs and outputs"),
        ("feedback(B,A)", "compatible I/O sizes"),
        ("feedback(A,A,1)", "singular algebraic loop"),
        ("feedback(tf(A),tf(A),1)", "singular algebraic loop"),
        ("feedback(tf(1),tf(1),1)", "singular algebraic loop"),
        ("feedback(A,A,0)", "sign must be +1 or -1"),
        ("[A;B]", "same number of inputs"),
        ("[A;A] + [A A]", "same number of inputs and outputs"),
        ("[[A;A] A]", "same number of outputs"),
        ("series(A,B,[1 2],1)", "same length"),
        ("series(A,B,3,1)", "valid port indices"),
        ("parallel(A,B,[1 1],[2 3],1,1)", "distinct port indices"),
        ("feedback(A,A,1,1)", "compatible I/O sizes"),
    ] {
        let error = e.execute(code).unwrap_err();
        assert!(error.contains(text), "{code}: {error}");
    }
}

#[test]
fn control_mimo_feedback_rejects_dependent_loop_columns() {
    let mut e = Engine::default();
    // python-control rejects I + D = [1 1; 49 49] as singular.
    let error = e
        .execute("R=feedback(ss([0 1;49 48]),eye(2));")
        .unwrap_err();
    assert!(
        error.contains("Feedback has a singular algebraic loop"),
        "{error}"
    );
    e.execute(concat!(
        "R=feedback(ss(eye(2)),diag([0 1e16]));",
        "assert(norm(R.D-diag([1 1e-16]))<1e-12);",
    ))
    .unwrap();
}

#[test]
fn control_mimo_series_expands_a_scalar_filter() {
    let mut e = Engine::default();
    // The documented ZPK cascade applies H(s) = s/((s+1)*(s-7)) to each port.
    e.execute(concat!(
        "G=ss(eye(2)); H=zpk(0,[-1 7],1); R=series(G,H);",
        "assert(isa(R,'ss') && isequal(size(R),[2 2]));",
        "assert(norm(evalfr(R,1)+eye(2)/12)<1e-12);",
        "M=ss([1 2 3;4 5 6]);",
        "assert(norm(evalfr(series(M,H),1)+M.D/12)<1e-12);",
        "assert(norm(evalfr(series(H,M),1)+M.D/12)<1e-12);",
    ))
    .unwrap();
}

#[test]
fn control_mimo_concatenates_improper_rational_models() {
    let mut e = Engine::default();
    // The documented block diagonal includes the improper channel s.
    e.execute(concat!(
        "s=tf('s'); R=append(s,1);",
        "assert(isa(R,'tf') && isequal(size(R),[2 2]));",
        "assert(isequal(evalfr(R,3),[3 0;0 1]));",
        "assert(isequal(evalfr([s s],3),[3 3]));",
        "assert(isequal(evalfr([s;s],3),[3;3]));",
        "Z=append(s,zpk([],[],1)); assert(isa(Z,'zpk'));",
        "assert(isequal(evalfr(Z,3),[3 0;0 1]));",
        "F=tf([1 2],[1 0.5],0.2,'Variable','z^-1');",
        "J=append(1,F); assert(strcmp(J.Variable,'z^-1') && J.Ts==0.2);",
        "assert(norm(evalfr(J,2)-diag([1 1.6]))<1e-12);",
    ))
    .unwrap();
}

#[test]
fn control_mimo_evalfr_rejects_poles_and_empty_ports() {
    let mut e = Engine::default();
    // Match the published evalfr contract and the existing SISO diagnostics.
    for (code, expected) in [
        (
            "evalfr(tf({1,1},{[1 0],1}),0)",
            "Frequency coincides with a system pole",
        ),
        (
            "evalfr(tf(),1)",
            "Frequency response requires at least one input and one output",
        ),
        (
            "G=tf({1,1},1); evalfr(G([],:),1)",
            "Frequency response requires at least one input and one output",
        ),
        (
            "evalfr(tf({[1 0 0],1},1),1e200)",
            "Frequency response overflowed",
        ),
    ] {
        let error = e.execute(code).unwrap_err();
        assert!(error.contains(expected), "{code}: {error}");
    }
}

#[test]
fn control_mimo_plot_port_labels_and_shared_time_limits() {
    let setup = concat!(
        "G=ss(-1,[1 2],[3;4],0); G.InputName={'command';''};",
        "G.OutputName={'';'velocity'}; F=tf(G);"
    );
    for (code, rows) in [
        ("step(G,0:.5:1);", 2),
        ("impulse(F,0:.5:1);", 2),
        ("bode(F,[.2 1 2]);", 4),
    ] {
        let mut e = Engine::default();
        e.execute(setup).unwrap();
        e.execute(code)
            .unwrap_or_else(|err| panic!("{code}: {err}"));
        let fig = e.figure.as_ref().unwrap();
        let axes: BTreeMap<_, _> = fig
            .all_axes()
            .map(|(position, axes)| {
                assert_eq!((position.rows, position.columns), (rows, 2));
                ((position.top, position.left), axes)
            })
            .collect();
        assert_eq!(axes.len(), usize::from(rows) * 2);
        assert_eq!(axes[&(0, 0)].title, "From: command");
        assert_eq!(axes[&(0, 1)].title, "From: In(2)");
        assert!(axes[&(0, 0)].ylabel.starts_with("To: Out(1)"));
        assert!(axes[&(rows / 2, 0)].ylabel.starts_with("To: velocity"));
        if rows == 2 {
            e.execute("xlim([.25 .75]);").unwrap();
            assert!(
                e.figure
                    .as_ref()
                    .unwrap()
                    .all_axes()
                    .all(|(_, ax)| { ax.xlim == Some([0.25, 0.75]) })
            );
        }
    }
}
