//! Reference corpus replay. The fixture is a set of expected values.
//! LONG markers come from the source corpus; each case gets a fresh engine.
use matrust::Engine;

const REFERENCE: &str = include_str!("fixtures/fn/linalg2-expected.txt");
const SOURCE: &str = include_str!("fixtures/fn/linalg2-cases.txt");
/// Dependencies outside this task. These entries are checked to fail with the documented error.
const KNOWN: &[(usize, &str)] = &[
    (
        57,
        "'sparse' is a MATLAB function that MatRust does not support yet.",
    ),
    (
        104,
        "spline(x,y) would return a piecewise polynomial structure, which is not supported; call spline(x,y,xx)",
    ),
];
/// Every relaxed comparison is listed here; unlisted cases are byte-for-byte.
#[derive(Clone, Copy)]
enum Rule {
    ErrorLocation,
    Warning,
    Numeric { atol: f64, rtol: f64 },
    CharacterPadding,
}
const ROUNDING: Rule = Rule::Numeric {
    atol: 8e-15,
    rtol: 4e-15,
};
const HARNESS: &[(usize, Rule, &str)] = &[
    (
        13,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        20,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        26,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        33,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        41,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        48,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        55,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        61,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        71,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        75,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        80,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        85,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        90,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        95,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        98,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        113,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        125,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        129,
        Rule::ErrorLocation,
        "Compare the error message after removing the engine source-location prefix.",
    ),
    (
        9,
        ROUNDING,
        "Kernel roundoff: hess reflector updates differ from LAPACK dgehrd in the last bit (14.8 shows as ...97 vs ...01). abs <= 8e-15 + 4e-15*abs(expected) on displayed numbers and the stored result; layout, labels, class and size stay exact.",
    ),
    (
        58,
        ROUNDING,
        "Kernel roundoff: the inverse 1-norm comes from LU column solves, MATLAB's from normest1 on the dgetrf factors (6.85 vs 6.849999999999997). abs <= 8e-15 + 4e-15*abs(expected) on displayed numbers and the stored result; layout, labels, class and size stay exact.",
    ),
    (
        109,
        ROUNDING,
        "Kernel roundoff: residues come from one linear system, residue.m's from resi2 polynomial evaluation, so the double pole's residues are off by a few ulps and print in long form. abs <= 8e-15 + 4e-15*abs(expected) on displayed numbers and the stored result; class and size stay exact.",
    ),
    (
        32,
        Rule::Warning,
        "Keep the warning message and result; omit oracle backtrace and backspace decoration.",
    ),
    (
        53,
        Rule::Numeric {
            atol: 2e-17,
            rtol: 0.,
        },
        "Exactly singular magic(4): LU roundoff gives backend-dependent positive values near zero. Require a nonnegative result within 2e-17 of the oracle.",
    ),
    (
        120,
        Rule::CharacterPadding,
        "Render NUL character padding as spaces, matching the corpus console. Keep the stored block diagonal padding at character code zero.",
    ),
];

struct Case<'a> {
    id: usize,
    statement: &'a str,
    expected: &'a str,
    class: Option<(&'a str, usize, usize)>,
    long: bool,
}
fn cases() -> Vec<Case<'static>> {
    let source: Vec<_> = SOURCE.lines().collect();
    REFERENCE
        .split("@@CASE ")
        .skip(1)
        .map(|block| {
            let (header, body) = block.split_once('\n').unwrap();
            let (id, statement) = header.split_once('|').unwrap();
            let id: usize = id.parse().unwrap();
            let body = body.split_once("@@END").unwrap().0;
            let (expected, class) = if let Some((text, meta)) = body.split_once("@@CLASS ") {
                let (class, shape) = meta.trim().split_once(' ').unwrap();
                let dims: Vec<usize> = shape
                    .trim_matches(['[', ']'])
                    .split_whitespace()
                    .map(|s| s.parse().unwrap())
                    .collect();
                assert_eq!(dims.len(), 2);
                (text, Some((class, dims[0], dims[1])))
            } else {
                (body, None)
            };
            let long = source[id - 1].starts_with("LONG ");
            assert_eq!(
                source[id - 1]
                    .strip_prefix("LONG ")
                    .unwrap_or(source[id - 1]),
                statement
            );
            Case {
                id,
                statement,
                expected,
                class,
                long,
            }
        })
        .collect()
}
fn error_message(error: &str) -> &str {
    if error.starts_with("Line ") {
        error.split_once(": ").map_or(error, |(_, message)| message)
    } else {
        error
    }
}
fn normalize_warning(s: &str) -> String {
    let s = s.replace('\u{8}', "");
    s.lines()
        .filter(|l| !l.starts_with("[> In ") && !l.starts_with("  In "))
        .map(|l| {
            if l.contains("Warning:") {
                format!(
                    "Warning: {}",
                    l.split_once("Warning: ")
                        .unwrap()
                        .1
                        .trim_end_matches([' ', ']'])
                )
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}
/// These entries contain only real x results. Preserve each printed row and all
/// nonnumeric labels, and also check unrounded stored values against the reference.
fn numeric_match(actual: &str, expected: &str, e: &Engine, atol: f64, rtol: f64) -> bool {
    fn parse(s: &str) -> (Vec<Vec<String>>, Vec<f64>) {
        let mut values = Vec::new();
        let layout = s
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                line.split_whitespace()
                    .map(|token| {
                        if let Ok(value) = token.parse::<f64>() {
                            values.push(value);
                            "#".into()
                        } else {
                            token.to_string()
                        }
                    })
                    .collect()
            })
            .collect();
        (layout, values)
    }
    let (al, av) = parse(actual);
    let (el, ev) = parse(expected);
    let close =
        |a: f64, b: f64| a.is_finite() && b.is_finite() && (a - b).abs() <= atol + rtol * b.abs();
    if al != el || av.len() != ev.len() || !av.iter().zip(&ev).all(|(&a, &b)| close(a, b)) {
        return false;
    }
    let Some(array) = e.array("x") else {
        return false;
    };
    let Some(x) = array.as_numeric() else {
        return false;
    };
    x.len() == ev.len()
        && (0..x.rows()).all(|i| (0..x.cols()).all(|j| close(x.get(i, j), ev[i * x.cols() + j])))
}

#[test]
fn corpus_reference() {
    let cases = cases();
    assert_eq!(cases.len(), 129);
    for id in KNOWN.iter().map(|v| v.0).chain(HARNESS.iter().map(|v| v.0)) {
        assert!(cases.iter().any(|c| c.id == id));
    }
    let mut failures = Vec::new();
    let mut passed = 0;
    for c in cases {
        let mut e = Engine::default();
        if c.long {
            e.execute("format long").unwrap();
        }
        let rule = HARNESS.iter().find(|v| v.0 == c.id).map(|v| v.1);
        let result = e.execute(c.statement);
        if let Some(&(_, reason)) = KNOWN.iter().find(|&&(id, _)| id == c.id) {
            assert_eq!(
                error_message(&result.unwrap_err()),
                reason,
                "KNOWN case {} changed",
                c.id
            );
            continue;
        }
        let actual = match result {
            Ok(r) => r
                .output
                .iter()
                .map(|s| format!("{s}\n"))
                .collect::<String>(),
            Err(err) => format!(
                "ERR {}\n",
                if matches!(rule, Some(Rule::ErrorLocation)) {
                    error_message(&err)
                } else {
                    &err
                }
            ),
        };
        let (actual, expected) = if matches!(rule, Some(Rule::Warning)) {
            (normalize_warning(&actual), normalize_warning(c.expected))
        } else {
            (actual, c.expected.to_string())
        };
        let class = e
            .array("x")
            .map(|x| (x.class().to_string(), x.rows(), x.cols()));
        let class_ok = c
            .class
            .is_none_or(|(cl, r, col)| class == Some((cl.to_string(), r, col)));
        let text_ok = match rule {
            Some(Rule::Numeric { atol, rtol }) => {
                numeric_match(&actual, &expected, &e, atol, rtol)
                    && (c.id != 53 || e.array("x").unwrap().as_numeric().unwrap().data()[0] >= 0.)
            }
            Some(Rule::CharacterPadding) => actual.replace('\0', " ") == expected,
            _ => actual == expected,
        };
        if text_ok && class_ok {
            passed += 1;
        } else {
            failures.push(format!(
                "case {} `{}`\nexpected {:?}\nactual   {:?}\nclass {:?}, expected {:?}",
                c.id, c.statement, expected, actual, class, c.class
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{passed}/{} passed; {} KNOWN; {} failures:\n{}",
        129 - KNOWN.len(),
        KNOWN.len(),
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn numerical_identities() {
    let mut e = Engine::default();
    for code in [
        "A=[1 2 3;4 5 6;7 8 10]; [Q,H]=hess(A); x=norm(A-Q*H*Q','fro');",
        "A=[1 2 3;4 5 6;7 8 10]; [Q,T]=schur(A); x=norm(A-Q*T*Q','fro');",
        "A=[1 2 3;2 1 4;3 4 1];[L,D,P]=ldl(A);x=norm(P'*A*P-L*D*L','fro');",
        "A=[1 2;3 4];B=[5 1;2 6];C=[7 8;9 10];X=sylvester(A,B,C);x=norm(A*X+X*B-C,'fro');",
        "A=[1 100 10000;0.01 1 100;0.0001 0.01 1];[T,B]=balance(A);x=norm(A*T-T*B,'fro');",
        "[U,T]=schur([1 2;-3 1]);[V,D]=rsf2csf(U,T);x=max(max(abs(V*D*V'-[1 2;-3 1])));",
        "[r,p,k]=residue([1 4 6],[1 3 2]);[b,a]=residue(r,p,k);x=norm(b-[1 4 6]);",
        "A=[1+2i 3;4i 5-i];[U,T]=schur(A);x=max(max(abs(A-U*T*U')));",
        "A=[1+i 2 3i;4 5-i 6;7i 8 10];[Q,H]=hess(A);x=max(max(abs(A-Q*H*Q')));",
        "A=[1 2 3;2 1 4;3 4 1];[U,D,P]=ldl(A,'upper');x=norm(P'*A*P-U'*D*U,'fro');",
        "A=[1 2 3;2 1 4;3 4 1];[L,D]=ldl(A);x=norm(A-L*D*L','fro');",
        "V=[1 1;i -i];D=[i 0;0 -i];[Vr,Dr]=cdf2rdf(V,D);x=norm([0 1;-1 0]*Vr-Vr*Dr,'fro');",
        "X=linsolve([1 1;2 2;3 3],[1;2;3]);x=norm(X-[1;0]);",
        "A=[1 0;0 1;1 1];b=[1;-1;1];[z,r,res,flag,out,lambda]=lsqnonneg(A,b);x=norm(z.*lambda);",
        "A=[0 0 0;2 3 1;0 0 5];[T,B]=balance(A);x=norm(A*T-T*B,'fro');",
        "A=[1 2;2 4];[c,v]=condest(A);x=norm(A*v)+abs(norm(v,1)-1);",
    ] {
        e.execute(code)
            .unwrap_or_else(|err| panic!("{code}: {err}"));
        let x = e.array("x").unwrap();
        assert!(x.as_numeric().unwrap().data()[0] < 1e-10, "{code}: {x:?}");
    }
}

fn scalar(e: &Engine, name: &str) -> f64 {
    e.array(name).unwrap().as_numeric().unwrap().data()[0]
}

/// Review rev1: checks use mathematical relations and reference values.
#[test]
fn review_relations() {
    let mut e = Engine::default();
    // Triangular flags on a rectangular A: no triangular kernel, n-by-k result.
    for (code, rows) in [
        ("x=linsolve(ones(3,2),ones(3,1),struct('LT',true));", 2),
        ("x=linsolve(ones(2,3),ones(2,1),struct('UT',true));", 3),
    ] {
        e.execute(code)
            .unwrap_or_else(|err| panic!("{code}: {err}"));
        let x = e.array("x").unwrap();
        assert_eq!((x.rows(), x.cols()), (rows, 1), "{code}");
    }
    // Pivoted QR basic solution and mldivide's rank warning.
    let run = e
        .execute("X=linsolve([1 1;2 2;3 3],[1;2;3]);x=norm(X-[1;0]);")
        .unwrap();
    assert!(
        run.output
            .concat()
            .contains("Warning: Rank deficient, rank = 1, tol ="),
        "{:?}",
        run.output
    );
    assert!(scalar(&e, "x") < 1e-14);
    // dgecon estimate below eps: the close-to-singular warning, after held text.
    let run = e
        .execute("fprintf('a'); x=linsolve([1 2;2 4.000000000000001],[1;2]);")
        .unwrap();
    let text = run.output.concat();
    assert!(
        text.starts_with("aWarning: Matrix is close to singular or badly scaled. Results may be inaccurate. RCOND ="),
        "{text:?}"
    );
    // dlanv2: a complex pair's 2x2 block has equal diagonals.
    e.execute("A=[1 2;-3 4];[U,T]=schur(A);x=norm(A-U*T*U','fro')+norm(U'*U-eye(2),'fro');")
        .unwrap();
    assert!(scalar(&e, "x") < 1e-14);
    let t = e.array("T").unwrap();
    let t = t.as_numeric().unwrap().data().to_vec();
    assert!(
        (t[0] - 2.5).abs() < 1e-14 && (t[3] - 2.5).abs() < 1e-14,
        "{t:?}"
    );
    assert!(t[1] * t[2] < 0., "{t:?}");
    // dsytrf with IDAMAX's first tie: the (1,2) 2x2 pivot, P = I.
    e.execute(
        "A=[0 1 1;1 0 1;1 1 0];[L,D,P]=ldl(A);x=norm(P-eye(3),'fro')+norm(P'*A*P-L*D*L','fro');",
    )
    .unwrap();
    assert!(scalar(&e, "x") < 1e-14);
    // Partial fractions of 1/((s-a)(s-b)) are +-1/(a-b): distinct close poles.
    e.execute("[r,p]=residue(1,poly([0.001 0.0015]));x=norm(sort(r)-[-2000;2000])/2000;")
        .unwrap();
    assert!(scalar(&e, "x") < 1e-6);
    // poly.m gives a real denominator for conjugate-closed poles.
    e.execute("p=[-1+2i;-1-2i;-0.3+0.7i;-0.3-0.7i];r=[1+i;1-i;2-3i;2+3i];[b,a]=residue(r,p,[]);x=double(isreal(a)&&isreal(b));")
        .unwrap();
    assert_eq!(scalar(&e, "x"), 1.);
    // zgehrd: complex Hessenberg form with a real subdiagonal.
    e.execute("A=[1 2 3;1i 5 6;2 8 1i];[Q,H]=hess(A);x=abs(imag(H(2,1)))+abs(imag(H(3,2)))+max(max(abs(A-Q*H*Q')));")
        .unwrap();
    assert!(scalar(&e, "x") < 1e-13);
    // balance's documented relations T(:,P) = diag(S), B(P,P) = diag(1./S)*A*diag(S).
    e.execute("A=[1 2 0;0 3 0;4 5 6];[S,P,B]=balance(A);[T,B2]=balance(A);T2(:,P)=diag(S);C(P,P)=diag(1./S)*A*diag(S);x=norm(T-T2,'fro')+norm(B-C,'fro')+norm(B-B2,'fro');")
        .unwrap();
    assert!(scalar(&e, "x") < 1e-14);
    assert_eq!(
        e.array("P").unwrap().as_numeric().unwrap().data(),
        &[2., 3., 1.]
    );
    // lsqnonneg.m: outer iteration count and a message field.
    e.execute("[z,rn,res,flag,out]=lsqnonneg([1 0;0 1],[1;2]);x=double(isfield(out,'message'))+out.iterations;")
        .unwrap();
    assert_eq!(scalar(&e, "x"), 3.);
    // Matrix Horner evaluation multiplies by X, then adds the next
    // coefficient to the diagonal; compare bit for bit.
    e.execute("A=[0.1 0.7;0.3 0.9];p=[0.3 1.1 2.7];Y=p(1)*eye(2);Y=A*Y+p(2)*eye(2);Y=A*Y+p(3)*eye(2);x=double(isequal(polyvalm(p,A),Y));")
        .unwrap();
    assert_eq!(scalar(&e, "x"), 1.);
    // Norm estimation warns after 100 iterations and returns its current estimate.
    let run = e
        .execute("[n,c]=normest(diag([1 0.99999]),1e-300);")
        .unwrap();
    assert!(
        run.output
            .concat()
            .contains("Warning: NORMEST did not converge for 100 iterations"),
        "{:?}",
        run.output
    );
    assert_eq!(scalar(&e, "c"), 101.);
    assert!((scalar(&e, "n") - 1.).abs() < 1e-4);
    // mkpp.m: k = 0 coefficients per piece is PPNumberMismatchCoeffs.
    assert!(e.execute("mkpp([0 1],[]);").is_err());
    // No pivots leave a 0x0 pivot-index result.
    e.execute("[R,jb]=rref(zeros(2));").unwrap();
    let jb = e.array("jb").unwrap();
    assert_eq!((jb.rows(), jb.cols()), (0, 0));
    // Equal poles reuse the previous quotient: O(n^2) inverse residue.
    e.execute("[b,a]=residue(ones(1,400),-ones(1,400),[]);")
        .unwrap();
    assert_eq!(e.array("a").unwrap().cols(), 401);
}

#[test]
fn input_limits_and_validation() {
    for code in [
        "rref([1 NaN])",
        "rref(eye(2),-1)",
        "normest(eye(2),0)",
        "mkpp([0 0 1],[1;2])",
        "mkpp([0 1],1,Inf)",
        "ppval(struct('form','pp','breaks',[0 1],'coefs',1,'pieces',1e300,'order',1,'dim',1),0)",
        "ppval(mkpp([0 1],[1;2],2),ones(2))",
        "linsolve(eye(2),ones(2,1),struct('LT',true,'UT',true))",
        "sylvester(1,-1,1)",
        "lscov(eye(2),ones(2,1),[-1;1])",
        "[x,se]=lscov([],[]);",
        "ldl(eye(2),1)",
        "balance(eye(2),1)",
    ] {
        assert!(Engine::default().execute(code).is_err(), "{code}");
    }
    let mut e = Engine::default();
    e.execute("A=ones(2000);").unwrap();
    let error = e.execute("hess(A);").unwrap_err();
    assert!(error.contains("work limit"), "{error}");
    e.execute("[x,r]=linsolve(zeros(0,3),zeros(0,1));").unwrap();
    assert_eq!(e.array("x").unwrap().rows(), 3);
    assert_eq!(e.array("r").unwrap().as_numeric().unwrap().data(), &[0.]);
    e.cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(e.execute("rref(eye(2))").is_err());
}

#[test]
fn sylvester_repeated_eigenvalues_and_real_schur_blocks() {
    let mut e = Engine::default();
    e.execute("A=eye(32)+reshape(sin(1:1024),32,32)/32;[Q,T]=schur(A);x=norm(A-Q*T*Q','fro');")
        .unwrap();
    assert!(e.array("x").unwrap().as_numeric().unwrap().data()[0] < 1e-10);
    for setup in [
        "A=eye(32)+reshape(sin(1:1024),32,32)/32;B=3*eye(32);C=ones(32);",
        "A=2*eye(3);B=[1 2;3 4];C=[1 2;3 4;5 6];",
        "A=[0 1;-1 0];B=[2 1;-3 2];C=[1 2;3 4];",
        "A=eye(32)+reshape(sin(1:1024),32,32)/32;B=diag(1:32);C=ones(32);",
    ] {
        e.execute(setup).unwrap();
        e.execute("X=sylvester(A,B,C);x=norm(A*X+X*B-C,'fro');")
            .unwrap();
        assert!(
            e.array("x").unwrap().as_numeric().unwrap().data()[0] < 1e-10,
            "{setup}"
        );
    }
}

#[test]
fn normest_restarts_when_initial_vector_is_in_the_nullspace() {
    for matrix in ["[1 -1]", "[1 -1;-1 1]"] {
        let mut e = Engine::default();
        e.execute(&format!("A={matrix};x=abs(normest(A)-norm(A));"))
            .unwrap();
        assert!(e.array("x").unwrap().as_numeric().unwrap().data()[0] < 1e-12);
    }
}

#[test]
fn block_diagonal_retains_integer_bits_and_concatenation_class() {
    let mut e = Engine::default();
    e.execute("A=blkdiag(uint64(9007199254740993),uint64(18446744073709551615));x=A(1,1)==uint64(9007199254740993);")
        .unwrap();
    assert_eq!(e.array("A").unwrap().class(), "uint64");
    let matrust::Array::Typed(a) = e.array("A").unwrap() else {
        unreachable!()
    };
    assert_eq!(a.int(0), 9_007_199_254_740_993);
    assert_eq!(a.int(3), 18_446_744_073_709_551_615);
    assert!(matches!(e.array("x"), Some(matrust::Array::Logical(ref a)) if a.data() == [1]));
    // blkdiag.m concatenates [A 0; 0 B] with double zero padding.
    for (blocks, padded) in [
        ("true,false", "[true 0;0 false]"),
        ("'a',int8(1)", "['a' 0;0 int8(1)]"),
        ("int8(1),2.5", "[int8(1) 0;0 2.5]"),
    ] {
        e.execute(&format!("x=blkdiag({blocks});y={padded};z=isequal(x,y);"))
            .unwrap();
        assert_eq!(
            e.array("x").unwrap().class(),
            e.array("y").unwrap().class(),
            "{blocks}"
        );
        assert!(matches!(e.array("z"), Some(matrust::Array::Logical(ref a)) if a.data() == [1]));
    }
    e.execute("x=blkdiag('a',1);").unwrap();
    let matrust::Array::Character(c) = e.array("x").unwrap() else {
        unreachable!()
    };
    assert_eq!(c.units(), &[97, 0, 0, 1]);
}

#[test]
fn lscov_variance_and_bounded_covariance() {
    let mut e = Engine::default();
    e.execute("[b,se,mse,S]=lscov([1 0;1 1;1 2],[1;2;4]);x=norm(S-[5/36 -1/12;-1/12 1/12],'fro');")
        .unwrap();
    assert!(e.array("x").unwrap().as_numeric().unwrap().data()[0] < 1e-12);
    e.execute("A=ones(1,5000);[b,se]=lscov(A,1);x=sum(b);")
        .unwrap();
    assert!((e.array("x").unwrap().as_numeric().unwrap().data()[0] - 1.).abs() < 1e-10);
    assert_eq!(e.array("se").unwrap().rows(), 5000);
    assert!(e.execute("[b,se,mse,S]=lscov(A,1);").is_err());
}

/// Fix round 1: extreme inputs give the exact result or a clean error, never a
/// panic, a silent zero or NaN.
#[test]
fn extreme_inputs_give_results_or_clean_errors() {
    let mut e = Engine::default();
    // residue: a denominator whose normalised coefficients overflow is an error.
    let error = e.execute("[r,p,k]=residue(1,[1e-310 1]);").unwrap_err();
    assert!(
        error_message(&error).contains("must not contain NaN or Inf"),
        "{error}"
    );
    // lsqnonneg: a valid variable far below the dual tolerance stays.
    e.execute("[x,rn,res,flag,out,lambda]=lsqnonneg(1e8,1);it=out.iterations;")
        .unwrap();
    assert_eq!(scalar(&e, "x"), 1e-8);
    assert_eq!(scalar(&e, "rn"), 0.);
    assert_eq!(scalar(&e, "flag"), 1.);
    assert_eq!(scalar(&e, "it"), 1.);
    // The reported dual is the true C'*(d - C*x), not a zeroed entry.
    assert!(scalar(&e, "lambda").abs() < 1e-6);
    e.execute("[x,rn,res,flag,out,lambda]=lsqnonneg([1;-1]*1e-6,[-1;1]);")
        .unwrap();
    assert_eq!(scalar(&e, "x"), 0.);
    assert!(scalar(&e, "lambda") < 0.);
    // residue: scaling a and b by 1e308 must not change r, p or k.
    e.execute(
        "[r,p,k]=residue(1e308,[1e308 0 -1e308]);[ps,ix]=sort(p);rs=r(ix);ok=double(isempty(k));",
    )
    .unwrap();
    for (name, want) in [("ps", [-1., 1.]), ("rs", [-0.5, 0.5])] {
        let v = e.array(name).unwrap();
        let v = v.as_numeric().unwrap().data().to_vec();
        assert!(
            v.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1e-14),
            "{name} {v:?}"
        );
    }
    assert_eq!(scalar(&e, "ok"), 1.);
    // rref: the default tolerance never overflows to Inf and zeroes the rows.
    e.execute("R=rref([1e308 1e308]);").unwrap();
    assert_eq!(
        e.array("R").unwrap().as_numeric().unwrap().data(),
        &[1., 1.]
    );
    // polyvalm: an infinite coefficient scales the diagonal of I only.
    e.execute("Y=polyvalm(Inf,eye(2));").unwrap();
    let inf = f64::INFINITY;
    assert_eq!(
        e.array("Y").unwrap().as_numeric().unwrap().data(),
        &[inf, 0., 0., inf]
    );
}

#[test]
fn pp_coefficients_retain_shared_storage() {
    let mut e = Engine::default();
    e.execute("A=[1 0;2 1];pp=mkpp([0 1 2],A);C=pp.coefs;")
        .unwrap();
    assert!(e.array("A").unwrap().same_snapshot(&e.array("C").unwrap()));
    e.execute("C(1,1)=9;x=ppval(pp,0.5);").unwrap();
    assert_eq!(e.array("x").unwrap().as_numeric().unwrap().data(), &[0.5]);
}

/// Opt-in release benchmark: bounded pools, one warm-up, five measured calls.
/// The caller records CPU and machine limits alongside the printed samples.
#[test]
#[ignore = "release benchmark; run explicitly with --ignored --nocapture"]
fn benchmark_linalg_more() {
    use std::time::Instant;
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (label, setup, expression) in [
                (
                    "polyvalm n=64 degree=4",
                    "A=reshape(sin(1:4096),64,64)/64;",
                    "Y=polyvalm([1 2 3 4 5],A);",
                ),
                (
                    "polyvalm n=512 degree=4",
                    "A=reshape(sin(1:262144),512,512)/512;",
                    "Y=polyvalm([1 2 3 4 5],A);",
                ),
                (
                    "hess n=128",
                    "A=reshape(sin(1:16384),128,128);",
                    "[Q,H]=hess(A);",
                ),
                (
                    "schur n=64",
                    "A=reshape(sin(1:4096),64,64);",
                    "[Q,T]=schur(A);",
                ),
                (
                    "linsolve m=512 n=32",
                    "A=reshape(sin((1:16384).^2),512,32);b=cos((1:512)');",
                    "x=linsolve(A,b);",
                ),
                (
                    "rcond n=128",
                    "A=eye(128)+reshape(sin(1:16384),128,128)/128;",
                    "x=rcond(A);",
                ),
                (
                    "rref m=128 n=64",
                    "A=reshape(sin((1:8192).^2),128,64);",
                    "Y=rref(A);",
                ),
                ("hess complex n=64", "A=reshape(sin(1:4096)+i*cos(1:4096),64,64);", "H=hess(A);"),
                ("schur complex n=32", "A=reshape(sin(1:1024)+i*cos(1:1024),32,32);", "[Q,T]=schur(A);"),
                ("ldl n=128", "S=reshape(sin(1:16384),128,128)/128;A=eye(128)+S+S';", "[L,D,P]=ldl(A);"),
                ("lscov m=512 n=32", "A=reshape(sin((1:16384).^2),512,32);b=cos((1:512)');w=linspace(1,2,512)';", "[x,se,mse]=lscov(A,b,w);"),
                ("lsqnonneg m=128 n=12", "A=reshape(sin((1:1536).^2),128,12);b=cos((1:128)');", "x=lsqnonneg(A,b);"),
                ("condest n=128", "A=eye(128)+reshape(sin(1:16384),128,128)/128;", "[c,v]=condest(A);"),
                ("normest n=128", "A=reshape(sin((1:16384).^2),128,128);", "[n,c]=normest(A);"),
                ("subspace m=128 n=8,4", "A=reshape(sin((1:1024).^2),128,8);B=A(:,1:4);", "x=subspace(A,B);"),
                ("sylvester n=32", "A=eye(32)+reshape(sin(1:1024),32,32)/32;B=3*eye(32);C=ones(32);", "X=sylvester(A,B,C);"),
                ("sylvester general n=32 repeated eigenvalues", "A=eye(32)+reshape(sin(1:1024),32,32)/32;B=diag(1:32);C=ones(32);", "X=sylvester(A,B,C);"),
                ("balance n=64", "d=2.^linspace(-8,8,64);A=diag(d)*(eye(64)+reshape(sin(1:4096),64,64)/64)*diag(1./d);", "[T,B]=balance(A);"),
                ("planerot n=2", "v=[3;4];", "[G,y]=planerot(v);"),
                ("mkpp pieces=128 order=4", "b=0:128;c=reshape(sin(1:512),128,4);", "pp=mkpp(b,c);"),
                ("unmkpp pieces=128 order=4", "pp=mkpp(0:128,reshape(sin(1:512),128,4));", "[b,c,l,k,d]=unmkpp(pp);"),
                ("ppval pieces=128 queries=4096", "pp=mkpp(0:128,reshape(sin(1:512),128,4));t=linspace(-1,129,4096);", "y=ppval(pp,t);"),
                ("residue degree=8", "a=poly(-1:-1:-8);b=[1 2 3];", "[r,p,k]=residue(b,a);"),
                ("residue inverse degree=8", "[r,p,k]=residue([1 2 3],poly(-1:-1:-8));", "[b,a]=residue(r,p,k);"),
                ("blkdiag two n=128 blocks", "A=reshape(sin(1:16384),128,128);", "B=blkdiag(A,A);"),
                ("cdf2rdf n=2", "V=[1 1;i -i];D=[i 0;0 -i];", "[Vr,Dr]=cdf2rdf(V,D);"),
                ("rsf2csf n=2", "U=eye(2);T=[1 2;-3 1];", "[Uc,Tc]=rsf2csf(U,T);"),
            ] {
                let mut e = Engine::default();
                e.execute(setup).unwrap();
                e.execute(expression).unwrap();
                let mut samples = Vec::new();
                for _ in 0..5 {
                    let start = Instant::now();
                    e.execute(expression).unwrap();
                    samples.push(start.elapsed().as_secs_f64() * 1000.);
                }
                println!("workers={workers} {label}: milliseconds={samples:?}");
            }
        });
    }
}

#[test]
fn rev2_matlab_answers() {
    let mut e = Engine::default();
    // linsolve LT: only the flagged triangle is read, also for the rcond output.
    for a in ["[2 5;1 3]", "[2 0;1 3]"] {
        e.execute(&format!(
            "opts=struct('LT',true);[x,r]=linsolve({a},[4;5],opts);y=norm(x-[2;1]);"
        ))
        .unwrap();
        assert_eq!(scalar(&e, "y"), 0., "{a}");
        assert_eq!(scalar(&e, "r"), 2. / 3., "{a}");
    }
    // planerot keeps r positive for [-4;3].
    e.execute("[G,y]=planerot([-4;3]);g=G(:)';y=y(:)';")
        .unwrap();
    let g = e.array("g").unwrap();
    assert_eq!(g.as_numeric().unwrap().data(), &[-0.8, -0.6, 0.6, -0.8]);
    let y = e.array("y").unwrap();
    assert_eq!(y.as_numeric().unwrap().data(), &[5., 0.]);
    // lsqnonneg outputs of the reference.
    e.execute("[x,rn,res,ef,outp]=lsqnonneg([1 0;0 1;1 1],[1;-1;2]);it=outp.iterations;al=outp.algorithm;ms=outp.message;")
        .unwrap();
    // The explicit-Q least squares matches the corpus digits (case 46); x of
    // these two examples can differ from the reference by a few ulps.
    let near = |e: &Engine, name: &str, want: &[f64]| {
        let x = e.array(name).unwrap();
        let x = x.as_numeric().unwrap().data().to_vec();
        assert!(
            x.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1e-15),
            "{x:?} vs {want:?}"
        );
    };
    near(&e, "x", &[1.4999999999999998, 0.]);
    assert_eq!(scalar(&e, "it"), 1.);
    e.execute("ok=double(strcmp(al,'active-set')&&strcmp(ms,'Optimization terminated.'));")
        .unwrap();
    assert_eq!(scalar(&e, "ok"), 1.);
    e.execute("C=[1 0 0;0 1 0;0 0 1;1 1 1;2 -1 3];d=[1;2;-1;0;5];[x,~,~,~,outp]=lsqnonneg(C,d);it=outp.iterations;")
        .unwrap();
    near(&e, "x", &[1.3529411764705885, 0., 0.41176470588235303]);
    assert_eq!(scalar(&e, "it"), 2.);
    // cdf2rdf (eig's [V,D] is unsupported for nonsymmetric A; V,D as the reference returns them) of a real matrix with a complex pair and a real eigenvalue.
    e.execute("A=[1 2 0;-2 1 0;0 0 3];s=sqrt(2)/2;V=[-s*i s*i 0;s s 0;0 0 1];D=[1+2i 0 0;0 1-2i 0;0 0 3];[Vr,Dr]=cdf2rdf(V,D);x=norm(A*Vr-Vr*Dr,'fro');")
        .unwrap();
    assert!(scalar(&e, "x") < 1e-14);
    let vr = e.array("Vr").unwrap();
    let vr = vr.as_numeric().unwrap().data().to_vec();
    let want = [
        0.,
        std::f64::consts::FRAC_1_SQRT_2,
        0.,
        -std::f64::consts::FRAC_1_SQRT_2,
        0.,
        0.,
        0.,
        0.,
        1.,
    ];
    for (g, w) in vr.iter().zip(want) {
        assert!((g - w).abs() <= 1e-15, "{vr:?}");
    }
    let dr = e.array("Dr").unwrap();
    let dr = dr.as_numeric().unwrap().data().to_vec();
    let want = [
        1.,
        -2.0000000000000004,
        0.,
        2.0000000000000004,
        1.,
        0.,
        0.,
        0.,
        3.,
    ];
    for (g, w) in dr.iter().zip(want) {
        assert!((g - w).abs() <= 1e-15, "{dr:?}");
    }
}
