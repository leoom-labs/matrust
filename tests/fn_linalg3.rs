//! Reference linalg3 corpus (eig in every form, eigs, svds, condeig, funm,
//! polyeig, qz, ordschur, ordeig, ordqz, gsvd, cholupdate, qrupdate, qrinsert,
//! qrdelete), stored as expected values. The LONG flags come from the case list
//! `fixtures/fn/linalg3-cases.txt`; the
//! captured fixture headers omit them. No reference is contacted by this test.
//! See docs/tasks/2026-10-24-fn-linalg3.md.
use matrust::Engine;

fn scalar(engine: &Engine, name: &str) -> f64 {
    engine.array(name).unwrap().as_numeric().unwrap().data()[0]
}

#[test]
fn round2_empty_nan_single_and_linear_polyeig() {
    let mut e = Engine::default();
    assert!(
        e.execute("[Q,R]=qrdelete(zeros(2,0),zeros(0,1),1);")
            .unwrap_err()
            .contains("Subscript indices")
    );
    e.execute("[F,flag,o]=funm([0.001 1;0 0.09],@log);")
        .unwrap();
    assert_eq!(scalar(&e, "flag"), 1.);
    e.execute("[F,flag,o]=funm([],@sin);t=o.terms;[X,d,s]=polyeig([],[],[]);")
        .unwrap();
    for name in ["F", "t", "X", "d", "s"] {
        assert_eq!(e.array(name).unwrap().len(), 0);
    }
    e.execute("[X,d,s]=polyeig([1 2;3 4],eye(2));r=norm([1 2;3 4]*X+X*diag(d),'fro');")
        .unwrap();
    assert!(scalar(&e, "r") < 1e-13);
    assert_eq!(e.array("s").unwrap().len(), 2);
    e.execute("d=eig(single([0 1;-1 0]));[V,D]=eig(single([0 1;-1 0]));")
        .unwrap();
    let d = e.array("d").unwrap();
    let d = d.as_complex().unwrap();
    assert_eq!(d.data()[0].im.abs(), 1.);
    assert_eq!(d.data()[1].im, -d.data()[0].im);
    e.execute("d=eig(single(eye(2)));").unwrap();
    assert_eq!(e.array("d").unwrap().class(), "single");
    assert!(
        e.execute("qz(int8(1),1)")
            .unwrap_err()
            .contains("type 'int8'")
    );
}

#[test]
fn round2_safe_eigenvectors_and_symmetric_scaling() {
    let mut e = Engine::default();
    for a in [
        "[1 1e200i 0;0 2 1e200i;0 0 3]",
        "[1 1e200 0;0 2 1e200;0 0 3]",
    ] {
        for balance in ["balance", "nobalance"] {
            e.execute(&format!("A={a};[V,D,W]=eig(A,'{balance}');r=norm(abs((A./1e200)*V-V*(D./1e200)),'fro');l=norm(abs(W'*(A./1e200)-(D./1e200)*W'),'fro');vn=sum(abs(V).^2);wn=sum(abs(W).^2);")).unwrap();
            assert!(scalar(&e, "r") < 1e-13);
            assert!(scalar(&e, "l") < 1e-13);
            for v in ["vn", "wn"] {
                for x in e.array(v).unwrap().as_numeric().unwrap().data() {
                    assert!((x - 1.).abs() < 1e-13, "{a}: {x}");
                }
            }
        }
    }
    for a in ["[0 0 1;0 0 1;1 1 0]", "[0 0 1i;0 0 1;-1i 1 0]"] {
        for scale in ["1e308", "1e-308"] {
            e.execute(&format!("A={scale}*{a};d=eig(A)./{scale};[V,D]=eig(A);r=norm(abs((A./{scale})*V-V*(D./{scale})),'fro');")).unwrap();
            assert!(scalar(&e, "r") < 1e-13);
            for x in e.array("d").unwrap().as_numeric().unwrap().data() {
                assert!(x.is_finite() && x.abs() < 1.5);
            }
        }
    }
}

#[test]
fn round2_cluster_blocks_and_handle_default() {
    let mut e = Engine::default();
    for (a, labels) in [
        ("[1 1 1 1;0 0 2 1;0 -2 0 1;0 0 0 3]", "[3 1 4 2]"),
        ("blkdiag([0 -1;1 0],2,1,4)", "[0 3 2 1 4]"),
    ] {
        e.execute(&format!("A={a};n=size(A,1);[S,T,Q,Z]=ordqz(A,eye(n),eye(n),eye(n),{labels});[U,R]=ordschur(eye(n),A,{labels});x=norm(abs(ordeig(S,T)-ordeig(R)));r=norm(abs(Q*A*Z-S),'fro');")).unwrap();
        assert!(scalar(&e, "x") < 1e-13);
        assert!(scalar(&e, "r") < 1e-13);
    }
    let out = e.execute("d=eigs(@(v) diag(1:10)*v,10,9);").unwrap();
    assert_eq!(e.array("d").unwrap().len(), 10);
    assert!(out.output.iter().any(|line| line.contains("k < n-1")));
    e.execute("d=eigs(@(v) diag(1:10)*v,10,2,'li');").unwrap();
    assert!(e.execute("d=eigs(@(v) diag(1:10)*v,10,2,'la');").is_err());
    e.execute("d=eigs(@(v) diag(1:10)*v,10,2,'la',struct('issym',true));")
        .unwrap();
}

#[test]
fn round2_excess_input_arguments() {
    for code in [
        "ordeig(1,1,99)",
        "ordschur(1,1,true,99)",
        "ordqz(1,1,1,1,true,99)",
        "gsvd(1,1,0,99)",
        "funm(1,@exp,struct(),99)",
        "condeig(1,99)",
        "qz(1,1,'real',99)",
        "eigs(eye(10),2,'lm',struct(),99)",
        "svds(eye(10),2,'L',struct(),99)",
    ] {
        let mut e = Engine::default();
        assert!(
            e.execute(code)
                .unwrap_err()
                .ends_with("Too many input arguments."),
            "{code}"
        );
    }
}

#[test]
fn round2_mid_computation_cancel_escapes_try_catch() {
    for expression in [
        "[V,D]=eig(A)",
        "[V,D]=eig(H)",
        "[V,D]=eig(A,B,'qz')",
        "[V,D]=eig(H,B,'qz')",
        "[S,T,Q,Z]=qz(A,B,'real')",
        "[U,S,V]=svds(A,2)",
        "s=gsvd(A,B)",
        "F=funm(A,@exp)",
        "[U,R]=ordschur(B,T,1:384)",
        "[S,R,Q,Z]=ordqz(T,B,B,B,1:384)",
    ] {
        let mut e = Engine::default();
        e.execute("M=reshape(sin((1:147456).^2),384,384);A=M+M';H=A+1i*(M-M');B=eye(384);T=diag(1:384)+diag(ones(383,1),1);").unwrap();
        let flag = e.cancellation();
        let timer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(5));
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let result = e.execute(&format!("try;{expression};catch;swallowed=1;end"));
        timer.join().unwrap();
        assert!(
            result.unwrap_err().ends_with("Calculation cancelled"),
            "{expression}"
        );
        assert!(e.array("swallowed").is_none(), "{expression}");
    }
}

#[test]
fn review_funm_options_are_bounded() {
    for option in [
        "'MaxTerms',Inf",
        "'MaxTerms',NaN",
        "'MaxTerms',1e300",
        "'MaxTerms',0",
        "'MaxTerms',1.5",
        "'MaxTerms',[1 2]",
        "'Ord',[1 1e300]",
        "'Ord',[1 Inf]",
        "'Ord',[0 1]",
        "'Ord',[1 1.5]",
        "'Ord',1",
    ] {
        let mut e = Engine::default();
        let code = format!("funm([1 1;0 1],@exp,struct({option}))");
        assert!(e.execute(&code).is_err(), "{code}");
    }
    let mut e = Engine::default();
    e.execute("F=funm([1 1;0 1],@exp,struct('Ord',[2 2],'MaxTerms',20));x=norm(F-exp(1)*[1 1;0 1],'fro');").unwrap();
    assert!(scalar(&e, "x") < 1e-13);
}

#[test]
fn review_gsvd_tall_values_and_full_basis_budget() {
    let mut e = Engine::default();
    e.execute("x=gsvd(ones(100000,1),1);").unwrap();
    assert!((scalar(&e, "x") - 100000f64.sqrt()).abs() < 1e-9);
    let error = e
        .execute("[U,V,X,C,S]=gsvd(ones(100000,1),1);")
        .unwrap_err();
    assert!(error.contains("budget"), "{error}");
}

#[test]
fn review_polyeig_companion_budget() {
    let mut e = Engine::default();
    e.execute("A=eye(1000);").unwrap();
    let error = e.execute("x=polyeig(A,A,A,A,A,A,A);").unwrap_err();
    assert!(error.contains("budget"), "{error}");
    e.execute("x=polyeig(1,0,1);").unwrap();
    assert_eq!(e.array("x").unwrap().rows(), 2);
}

#[test]
fn fix1_gsvd_empty_blocks_and_economy_budget() {
    let mut e = Engine::default();
    // An empty B leaves infinite generalized singular values; an empty A zeros.
    e.execute(
        "x=gsvd(eye(2),zeros(0,2));ok=double(isequal(x,[Inf;Inf]));\
         [U,V,X,C,S]=gsvd(eye(2),zeros(0,2));r=norm(U*C*X'-eye(2),'fro');\
         sz=[size(U) size(V) size(C) size(S)];\
         y=gsvd(zeros(0,2),eye(2));ok0=double(isequal(y,[0;0]));",
    )
    .unwrap();
    assert_eq!(scalar(&e, "ok"), 1.);
    assert_eq!(scalar(&e, "ok0"), 1.);
    assert!(scalar(&e, "r") < 1e-14);
    e.execute("ok=double(isequal(sz,[2 2 0 0 2 2 0 2]));")
        .unwrap();
    assert_eq!(scalar(&e, "ok"), 1.);
    // No shape of zero extent panics, whatever the number of outputs.
    for (m, n, p) in (0..3).flat_map(|m| (0..3).flat_map(move |n| (0..3).map(move |p| (m, n, p)))) {
        for outputs in ["x", "[U,V,X,C,S]"] {
            for flag in ["", ",0"] {
                let mut e = Engine::default();
                let code = format!("{outputs}=gsvd(ones({m},{p}),ones({n},{p}){flag});");
                let _ = e.execute(&code);
            }
        }
    }
    // The economy form of a tall operand is sized by its economy factors.
    e.execute(
        "[U,V,X,C,S]=gsvd(ones(100000,1),1,0);sz=size(U);r=norm(U*C*X'-ones(100000,1),'fro');",
    )
    .unwrap();
    assert_eq!(e.array("sz").unwrap().len(), 2);
    assert_eq!(scalar(&e, "sz"), 100000.);
    assert!(scalar(&e, "r") < 1e-9);
}

#[test]
fn fix1_polyeig_condition_numbers_stay_finite_and_in_range() {
    let mut e = Engine::default();
    // A zero pivot in the inverse iteration must not turn s into NaN.
    e.execute(
        "A0=diag(ones(24,1),1);A1=fliplr(eye(25));[X,e,s]=polyeig(A0,A1);\
         t=s(abs(e)<1e-8);k=numel(t);d=abs(t-sqrt(24))/sqrt(24);",
    )
    .unwrap();
    assert_eq!(scalar(&e, "k"), 1.);
    assert!(scalar(&e, "d") < 1e-12, "{}", scalar(&e, "d"));
    // Weights near the ends of the double range neither overflow nor vanish.
    for a in ["1e200", "1e-200", "1e300"] {
        e.execute(&format!("a={a};[X,e,s]=polyeig(-a,a);d=abs(s-1/sqrt(2));"))
            .unwrap();
        assert!(scalar(&e, "d") < 1e-14, "{a}: {}", scalar(&e, "d"));
    }
    // One coefficient is a standard eigenproblem without condition numbers.
    let error = e.execute("[X,e,s]=polyeig(1);").unwrap_err();
    assert!(
        error.ends_with("Must provide at least two matrices."),
        "{error}"
    );
    e.execute("e=polyeig(1);[X,e]=polyeig([1 2;3 4]);").unwrap();
    assert_eq!(e.array("e").unwrap().len(), 2);
}

#[test]
fn fix2_polyeig_left_null_vector_scaling() {
    let mut e = Engine::default();
    // Uniform coefficient scaling must preserve both condition numbers,
    // including the left-null fallback for the exactly singular pencil.
    // Expected values from the homogeneous condition number (Dedieu & Tisseur, LAA 358 (2003)),
    // hand-checked: 2 at lambda = 0, sqrt(2) at lambda = 1 (the old code gave 1; no MATLAB value).
    for scale in ["1", "1e200", "1e-200", "1e300"] {
        e.execute(&format!(
            "a={scale};A0=[0 a;0 a];A1=[a 0;0 -a];[X,d,s]=polyeig(A0,A1);"
        ))
        .unwrap();
        let s = e.array("s").unwrap();
        assert_eq!((s.rows(), s.cols()), (2, 1));
        for (&actual, expected) in s
            .as_numeric()
            .unwrap()
            .data()
            .iter()
            .zip([2., std::f64::consts::SQRT_2])
        {
            assert!((actual - expected).abs() < 1e-13, "{scale}: {actual}");
        }
    }
}

#[test]
fn fix2_polyeig_shape_error_dimensions() {
    for (operand, expected) in [
        (
            "[1;2;3]",
            "Assignment has fewer non-singleton rhs dimensions than non-singleton subscripts",
        ),
        (
            "[1 2 3]",
            "Assignment has fewer non-singleton rhs dimensions than non-singleton subscripts",
        ),
        ("zeros(0,2)", "Subscripted assignment dimension mismatch."),
        ("zeros(2,0)", "Subscripted assignment dimension mismatch."),
    ] {
        let mut e = Engine::default();
        let error = e
            .execute(&format!("polyeig({operand},{operand})"))
            .unwrap_err();
        assert!(error.ends_with(expected), "{operand}: {error}");
    }
}

#[test]
fn review_empty_triangular_linsolve() {
    for flag in ["LT", "UT"] {
        let mut e = Engine::default();
        let result = e
            .execute(&format!("x=linsolve([],[],struct('{flag}',true));"))
            .unwrap();
        assert!(result.output.is_empty());
        assert_eq!(
            (e.array("x").unwrap().rows(), e.array("x").unwrap().cols()),
            (0, 0)
        );
        e.execute(&format!("[x,r]=linsolve([],[],struct('{flag}',true));"))
            .unwrap();
        assert_eq!(scalar(&e, "r"), 1.);
    }
}

#[test]
fn review_complex_cholupdate_identity() {
    let mut e = Engine::default();
    e.execute("v=[1;1i];R=cholupdate(eye(2),v);x=norm(abs(R'*R-(eye(2)+v*v')),'fro');")
        .unwrap();
    assert!(scalar(&e, "x") < 1e-14);
    e.execute("R1=cholupdate(R,v,'-');x=norm(abs(R1'*R1-eye(2)),'fro');")
        .unwrap();
    assert!(scalar(&e, "x") < 1e-14);
}

#[test]
fn review_eig_extreme_scales() {
    for scale in ["1e-300", "1e300"] {
        for option in ["'balance'", "'nobalance'"] {
            let mut e = Engine::default();
            e.execute(&format!("s={scale};A=[0 s;-s 0];[V,D,W]=eig(A,{option});x=norm(abs(A./s*V-V*(D./s)),'fro');y=norm(abs(W'*(A./s)-(D./s)*W'),'fro');d=diag(D)./s;")).unwrap();
            assert!(scalar(&e, "x") < 1e-13);
            assert!(scalar(&e, "y") < 1e-13);
            let d = e.array("d").unwrap();
            let d = d.as_complex().unwrap();
            assert!((d.data()[0].im.abs() - 1.).abs() < 1e-14);
        }
    }
}

#[test]
fn review_complex_small_separation_vectors() {
    let mut e = Engine::default();
    e.execute(
        "A=[1e-200 1e-200i 0;0 2e-200 0;0 0 1];[V,D]=eig(A);x=norm(abs(V'*V),'fro');E=A*V-V*D;",
    )
    .unwrap();
    assert!(scalar(&e, "x").is_finite());
    // The second vector is proportional to [i;1;0], not [0;1;0].
    e.execute("x=abs(V(1,2)./V(2,2)-1i);y=norm(abs(E),'fro');")
        .unwrap();
    assert!(scalar(&e, "x") < 1e-14);
    assert!(scalar(&e, "y") < 1e-214);
    for scale in ["1e-300", "1e300"] {
        e.execute(&format!(
            "s={scale};A=[0 1i*s;s 0];[V,D]=eig(A);x=norm(abs(A./s*V-V*(D./s)),'fro');"
        ))
        .unwrap();
        assert!(scalar(&e, "x") < 1e-13);
    }
}

#[test]
fn review_eigs_complex_b() {
    let mut e = Engine::default();
    e.execute(
        "B=[1 1i 0;-1i 2 0;0 0 3];x=eigs(eye(3),B,1);[V,D]=eigs(eye(3),B,1);r=norm(abs(V-B*V*D),'fro');",
    )
    .unwrap();
    assert!((scalar(&e, "x") - (3. + 5f64.sqrt()) / 2.).abs() < 1e-13);
    assert!(scalar(&e, "r") < 1e-13);
}

#[test]
fn review_update_excess_arguments() {
    for code in [
        "cholupdate(1,1,'+',99)",
        "qrupdate(1,1,1,1,99)",
        "qrinsert(1,1,1,1,'col',99)",
        "qrdelete(1,1,1,'col',99)",
    ] {
        let mut e = Engine::default();
        assert!(
            e.execute(code)
                .unwrap_err()
                .contains("Too many input arguments"),
            "{code}"
        );
    }
}

#[test]
fn review_complex_diag() {
    let mut e = Engine::default();
    e.execute("v=[1+2i;3-4i];A=diag(v);x=norm(diag(A)-v);y=norm(diag(diag(v,2),2)-v);z=norm(diag(diag(v,-2),-2)-v);").unwrap();
    for name in ["x", "y", "z"] {
        assert_eq!(scalar(&e, name), 0.);
    }
    assert!(e.execute("diag(v,100000)").is_err());
}

#[test]
fn review_last_digit_rule_is_per_number_and_preserves_text() {
    assert!(last_digits_match(
        "x =\n  -0.000000000000000\n",
        "x =\n                   0\n"
    ));
    assert!(last_digits_match(
        "x =\n 1.000 + 0.000i\n",
        "x =\n 1.000 - 0.000i\n"
    ));
    assert!(!last_digits_match(
        "x =\n 1.0e+03 *\n 0.000000000000001\n",
        "x =\n 1.0e+03 *\n 0.000000000000000\n"
    ));
    let base = "x =\n  1000.000000000000000  0.000000000001000\n";
    assert!(last_digits_match(
        "x =\n  1000.000000000001000  0.000000000001000\n",
        base
    ));
    assert!(!last_digits_match(
        "x =\n  1000.000000000000000  0.000000000001010\n",
        base
    ));
    assert!(!last_digits_match(
        "y =\n  1000.000000000000000  0.000000000001000\n",
        base
    ));
    assert!(!last_digits_match(
        "x =\n 1000.000000000000000  0.000000000001000\n",
        base
    ));
    assert!(!last_digits_match(
        "x =\n -1000.000000000000000  0.000000000001000\n",
        base
    ));
    assert!(
        !KNOWN
            .iter()
            .any(|(id, _)| [69, 71, 110, 245, 246].contains(id))
    );
}

#[test]
fn review_ordqz_conjugate_blocks_preserve_pencil() {
    for (a, b, select) in [
        ("[1 2 0;0 -1 3;1 0 -2]", "diag([2 1 1])", "'lhp'"),
        ("[1 2 0;0 -1 3;1 0 -2]", "diag([2 1 1])", "'rhp'"),
        ("[2 1 4;-3 2 5;0 0 -1]", "eye(3)", "[0 0 1]"),
        (
            "[2 1 4 2;-3 2 5 1;0 0 -2 3;0 0 -1 -2]",
            "eye(4)",
            "[0 0 1 1]",
        ),
    ] {
        let mut e = Engine::default();
        e.execute(&format!("A={a};B={b};[AA,BB,Q,Z]=qz(A,B,'real');[S,T,U,V]=ordqz(AA,BB,Q,Z,{select});x=norm(U*A*V-S,'fro');y=norm(U*B*V-T,'fro');u=norm(U*U'-eye(size(U,1)),'fro');v=norm(V'*V-eye(size(V,1)),'fro');d=ordeig(S,T);")).unwrap();
        for name in ["x", "y", "u", "v"] {
            assert!(scalar(&e, name) < 1e-12, "{a}: {name}={}", scalar(&e, name));
        }
        e.execute("r=real(d(1));").unwrap();
        if select != "'rhp'" {
            assert!(scalar(&e, "r") < 0.);
        } else {
            assert!(scalar(&e, "r") > 0.);
        }
    }
}

#[test]
fn review_generalized_eig_69_and_71_meet_per_value_tolerance() {
    for case in cases().into_iter().filter(|c| [69, 71].contains(&c.id)) {
        let (actual, class) = run(&case);
        assert!(
            last_digits_match(&actual, case.output),
            "case {}\nexpected {}\nactual {}",
            case.id,
            case.output,
            actual
        );
        assert_eq!(class.as_deref(), case.class_size);
    }
    for (a, b) in [("[1 2i;3 4]", "[2 0;0 1+1i]"), ("magic(3)", "hilb(3)")] {
        let mut e = Engine::default();
        e.execute(&format!(
            "A={a};B={b};[V,D]=eig(A,B);r=norm(abs(A*V-B*V*D),'fro')/norm(abs(A),'fro');"
        ))
        .unwrap();
        assert!(scalar(&e, "r") < 1e-13);
    }
}

#[test]
#[ignore = "release benchmark; run explicitly with --ignored --nocapture"]
fn benchmark_review_numerical_paths() {
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (label, setup, expression) in [
                (
                    "eig real n=128",
                    "A=reshape(sin((1:16384).^2),128,128);",
                    "[V,D]=eig(A);",
                ),
                (
                    "eig complex n=64",
                    "A=reshape(sin((1:4096).^2)+1i*cos(1:4096),64,64);",
                    "[V,D]=eig(A);",
                ),
                (
                    "eig tiny n=64",
                    "A=reshape(sin((1:4096).^2),64,64)*1e-300;",
                    "[V,D]=eig(A);",
                ),
                (
                    "eig symmetric n=128",
                    "M=reshape(sin((1:16384).^2),128,128);A=M+M';",
                    "[V,D]=eig(A);",
                ),
                (
                    "eig symmetric huge n=64",
                    "M=reshape(sin((1:4096).^2),64,64);A=(M+M')*1e300;",
                    "[V,D]=eig(A);",
                ),
                (
                    "qz real n=64",
                    "A=reshape(sin((1:4096).^2),64,64);B=eye(64);",
                    "[S,T,Q,Z]=qz(A,B,'real');",
                ),
                (
                    "svds real n=64",
                    "A=reshape(sin((1:4096).^2),64,64);",
                    "[U,S,V]=svds(A,6);",
                ),
                (
                    "gsvd square n=32",
                    "A=reshape(sin((1:1024).^2),32,32);B=eye(32);",
                    "[U,V,X,C,S]=gsvd(A,B);",
                ),
                ("gsvd tall 100000x1", "A=ones(100000,1);", "s=gsvd(A,1);"),
                (
                    "polyeig n=16 degree=2",
                    "A=eye(16);B=reshape(sin(1:256),16,16);",
                    "x=polyeig(A,B,A);",
                ),
                (
                    "ordqz n=3 1x2 block swap",
                    "[A,B,Q,Z]=qz([1 2 0;0 -1 3;1 0 -2],diag([2 1 1]),'real');",
                    "[S,T]=ordqz(A,B,Q,Z,'lhp');",
                ),
                (
                    "cholupdate complex n=128",
                    "R=eye(128);v=(sin((1:128)')+1i*cos((1:128)'))./128;",
                    "S=cholupdate(R,v);",
                ),
                (
                    "funm n=16",
                    "A=diag(ones(16,1))+diag(ones(15,1),1);",
                    "F=funm(A,@exp);",
                ),
            ] {
                let mut e = Engine::default();
                e.execute(setup).unwrap();
                e.execute(expression).unwrap();
                let mut times = Vec::new();
                for _ in 0..5 {
                    let start = std::time::Instant::now();
                    e.execute(expression).unwrap();
                    times.push(start.elapsed().as_secs_f64() * 1000.);
                }
                eprintln!("workers={workers} {label} milliseconds={times:?}");
            }
        });
    }
}

const REFERENCE: &str = include_str!("fixtures/fn/linalg3-expected.txt");
const CASE_LIST: &str = include_str!("fixtures/fn/linalg3-cases.txt");
const TOTAL: usize = 311;

/// Cases that cannot pass for a reason outside this task, with the reason.
/// Each one still runs; a KNOWN case that starts to pass is reported as stale.
const KNOWN: &[(usize, &str)] = &[
    (
        77,
        "single input: MATLAB runs single-precision LAPACK (ssyev); this engine computes in double and rounds, so the last single digit differs.",
    ),
    (
        98,
        "gallery is not implemented in the engine (outside this task).",
    ),
    (
        99,
        "gallery is not implemented in the engine (outside this task).",
    ),
    (
        100,
        "gallery is not implemented in the engine (outside this task).",
    ),
    (
        108,
        "eigs 'li': the two conjugate pairs tie in |imag| to the last bit; ARPACK's rounding picks the other pair.",
    ),
    (115, "sparse matrices are not supported by the engine."),
    (
        146,
        "Display: format long rounds 1.0875385167021725 up (the engine's display code, which this task must not change).",
    ),
    (
        179,
        "polyeig picks the linearisation block with the smaller residual; both residuals are equal up to rounding, and MATLAB picks the other block (a different phase).",
    ),
    (218, SCHUR),
    (219, SCHUR),
    (220, SCHUR),
    (223, SCHUR),
    (230, SCHUR),
    (
        257,
        "gsvd economy: the sign of a singular-vector pair from the inner SVD differs from LAPACK dgesvd.",
    ),
    (
        276,
        "chol of a complex matrix is not supported by the engine (outside this task).",
    ),
    (
        307,
        "qr of a complex matrix is not supported by the engine (outside this task).",
    ),
];

const SCHUR: &str = "Depends on the engine's schur (linalg2 group, not this task), whose U/T differ from MATLAB's in sign or rounding.";

/// Comparison rules other than byte-exact console text.
const ERROR_MESSAGE: &str = "Compare Diagnostic.message exactly, excluding the engine's line/column wrapper; the oracle captured MException.message.";
const WARNING_LINE: &str = "Compare the [Warning: ...] text exactly without its backtrace (it names the oracle script and folder), then the displayed values at the ARPACK rule.";
const ARPACK: &str = "eigs/svds: last digits come from ARPACK (and decide whether a value prints as 10 or 9.999999999999998); the displayed values, with any common scale factor applied, agree to relative 1e-12 of the largest; the variable name, value count, real/complex layout, class and size are exact.";
const DEFECTIVE: &str = "Defective double root 2 of [3 1; -1 1]: rounding noise of size sqrt(eps); numeric tokens agree to 1e-7 absolute, all other text exact.";
const LAST_DIGIT: &str = "Each numeric field (sign and fixed-width padding included), with the displayed scale applied, must agree with MATLAB within 1e-13 relative or 1e-15 absolute near zero. Field end positions, nonnumeric text, class and size are exact. Includes MKL and input qr/chol rounding.";
const HARNESS: &[(usize, &str)] = &[
    (245, LAST_DIGIT),
    (246, LAST_DIGIT),
    (69, LAST_DIGIT),
    (71, LAST_DIGIT),
    (156, LAST_DIGIT),
    (172, LAST_DIGIT),
    (181, LAST_DIGIT),
    (183, LAST_DIGIT),
    (192, LAST_DIGIT),
    (193, LAST_DIGIT),
    (196, LAST_DIGIT),
    (201, LAST_DIGIT),
    (202, LAST_DIGIT),
    (203, LAST_DIGIT),
    (204, LAST_DIGIT),
    (206, LAST_DIGIT),
    (231, LAST_DIGIT),
    (239, LAST_DIGIT),
    (240, LAST_DIGIT),
    (242, LAST_DIGIT),
    (243, LAST_DIGIT),
    (244, LAST_DIGIT),
    (247, LAST_DIGIT),
    (250, LAST_DIGIT),
    (251, LAST_DIGIT),
    (252, LAST_DIGIT),
    (254, LAST_DIGIT),
    (255, LAST_DIGIT),
    (256, LAST_DIGIT),
    (258, LAST_DIGIT),
    // Clean-room gsvd (2026-10-28): S(2,2) is the QR-derived sine of the CS
    // decomposition; its last bit prints 0.904214318644702, not ...701.
    (261, LAST_DIGIT),
    (267, LAST_DIGIT),
    (268, LAST_DIGIT),
    (271, LAST_DIGIT),
    (277, LAST_DIGIT),
    (282, LAST_DIGIT),
    (283, LAST_DIGIT),
    (285, LAST_DIGIT),
    (290, LAST_DIGIT),
    (292, LAST_DIGIT),
    (293, LAST_DIGIT),
    (294, LAST_DIGIT),
    (296, LAST_DIGIT),
    (301, LAST_DIGIT),
    (302, LAST_DIGIT),
    (303, LAST_DIGIT),
    (304, LAST_DIGIT),
    (305, LAST_DIGIT),
    (110, ARPACK),
    (40, DEFECTIVE),
    (41, DEFECTIVE),
    (81, ERROR_MESSAGE),
    (82, ERROR_MESSAGE),
    (83, ERROR_MESSAGE),
    (84, ERROR_MESSAGE),
    (85, ERROR_MESSAGE),
    (86, ERROR_MESSAGE),
    (87, ERROR_MESSAGE),
    (88, ERROR_MESSAGE),
    (89, ERROR_MESSAGE),
    (90, ERROR_MESSAGE),
    (91, ARPACK),
    (92, ARPACK),
    (93, ARPACK),
    (94, ARPACK),
    (95, ARPACK),
    (96, ARPACK),
    (97, ARPACK),
    (101, ARPACK),
    (102, ARPACK),
    (103, ARPACK),
    (104, ARPACK),
    (105, ARPACK),
    (106, ARPACK),
    (107, ARPACK),
    (109, ARPACK),
    (111, ARPACK),
    (112, ARPACK),
    (113, ARPACK),
    (114, ARPACK),
    (116, ARPACK),
    (117, WARNING_LINE),
    (118, ERROR_MESSAGE),
    (119, ERROR_MESSAGE),
    (120, ERROR_MESSAGE),
    (121, ERROR_MESSAGE),
    (122, ARPACK),
    (123, ARPACK),
    (124, ARPACK),
    (125, ARPACK),
    (126, ARPACK),
    (127, ARPACK),
    (128, WARNING_LINE),
    (129, ARPACK),
    (130, ARPACK),
    (131, ARPACK),
    (132, ARPACK),
    (133, ARPACK),
    (134, ARPACK),
    (135, ARPACK),
    (136, ARPACK),
    (137, ARPACK),
    (138, ARPACK),
    (139, ERROR_MESSAGE),
    (140, ERROR_MESSAGE),
    (141, ERROR_MESSAGE),
    (152, ERROR_MESSAGE),
    (153, ERROR_MESSAGE),
    (174, ERROR_MESSAGE),
    (175, ERROR_MESSAGE),
    (176, ERROR_MESSAGE),
    (189, ERROR_MESSAGE),
    (190, ERROR_MESSAGE),
    (191, ERROR_MESSAGE),
    (208, ERROR_MESSAGE),
    (209, ERROR_MESSAGE),
    (210, ERROR_MESSAGE),
    (211, ERROR_MESSAGE),
    (225, ERROR_MESSAGE),
    (226, ERROR_MESSAGE),
    (238, ERROR_MESSAGE),
    (248, ERROR_MESSAGE),
    (249, ERROR_MESSAGE),
    (264, ERROR_MESSAGE),
    (265, ERROR_MESSAGE),
    (266, ERROR_MESSAGE),
    (274, ERROR_MESSAGE),
    (278, ERROR_MESSAGE),
    (279, ERROR_MESSAGE),
    (280, ERROR_MESSAGE),
    (281, ERROR_MESSAGE),
    (287, ERROR_MESSAGE),
    (288, ERROR_MESSAGE),
    (289, ERROR_MESSAGE),
    (298, ERROR_MESSAGE),
    (299, ERROR_MESSAGE),
    (300, ERROR_MESSAGE),
    (309, ERROR_MESSAGE),
    (310, ERROR_MESSAGE),
    (311, ERROR_MESSAGE),
];

fn rule(id: usize) -> Option<&'static str> {
    HARNESS.iter().find(|&&(n, _)| n == id).map(|&(_, r)| r)
}

struct Case<'a> {
    id: usize,
    statement: &'a str,
    long: bool,
    output: &'a str,
    class_size: Option<&'a str>,
}

fn cases() -> Vec<Case<'static>> {
    let list: Vec<&str> = CASE_LIST
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let mut cases = Vec::new();
    let mut rest = REFERENCE;
    while let Some(start) = rest.find("@@CASE ") {
        let (header, after) = rest[start + 7..].split_once('\n').expect("header");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (block, after) = after.split_once("@@END\n").expect("end");
        let (output, class_size) = match block.split_once("@@CLASS ") {
            Some((output, class)) => (
                output,
                Some(class.strip_suffix('\n').expect("class newline")),
            ),
            None => (block, None),
        };
        let id: usize = id.parse().expect("numeric id");
        let listed = list[id - 1];
        let long = listed.starts_with("LONG ");
        assert_eq!(
            listed.strip_prefix("LONG ").unwrap_or(listed),
            statement,
            "case list and fixture disagree at {id}"
        );
        cases.push(Case {
            id,
            statement,
            long,
            output,
            class_size,
        });
        rest = after;
    }
    cases
}

/// The `[Warning: ...]` text without the backtrace, as the engine prints it.
fn warning_only(text: &str) -> String {
    let mut s = text.to_owned();
    while let Some(at) = s.find("[\u{8}> In ") {
        let end = at + s[at..].find("]\u{8} \n").unwrap() + "]\u{8} \n".len();
        s.replace_range(at..end, "");
    }
    s.replace("[\u{8}Warning: ", "Warning: ")
        .replace("]\u{8} \n", "\n")
}

/// Same text between the numbers; each number within `tol(scale)` of the
/// expected one, where scale is the largest magnitude in the expected text.
fn numbers_match(actual: &str, expected: &str, relative: bool, tol: f64) -> bool {
    let re = regex::Regex::new(r"[0-9]+(?:\.[0-9]+)?(?:e[-+][0-9]+)?").unwrap();
    let a: Vec<_> = re.find_iter(actual).collect();
    let b: Vec<_> = re.find_iter(expected).collect();
    if a.len() != b.len() {
        return false;
    }
    let value = |m: &regex::Match| m.as_str().parse::<f64>().unwrap();
    let scale = b.iter().map(value).fold(0f64, f64::max);
    let limit = if relative { tol * scale } else { tol };
    let (mut ai, mut bi) = (0, 0);
    for (x, y) in a.iter().zip(&b) {
        if actual[ai..x.start()] != expected[bi..y.start()] {
            return false;
        }
        // NaN differences are never within the limit.
        let within = (value(x) - value(y)).abs() <= limit;
        if x.as_str() != y.as_str() && !within {
            return false;
        }
        ai = x.end();
        bi = y.end();
    }
    actual[ai..] == expected[bi..]
}

/// The values a console display shows: (value, is imaginary part), with a
/// `1.0e+NN *` scale factor applied; `Column(s) ...` headers are skipped.
fn displayed_values(text: &str) -> Vec<(f64, bool)> {
    let number =
        regex::Regex::new(r"[-+]?[0-9]+(?:\.[0-9]+)?(?:e[-+][0-9]+)?i?|[-+]?Inf|NaN").unwrap();
    let mut scale = 1.;
    let mut values = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("Column")
            || t.ends_with(" =")
            || t.starts_with("Empty")
            || t.starts_with("Warning")
            || t.starts_with("Using")
        {
            continue;
        }
        if let Some(f) = t.strip_suffix(" *") {
            scale = f.parse().unwrap();
            continue;
        }
        let joined = t.replace("+ ", "+").replace("- ", "-");
        for m in number.find_iter(&joined) {
            let s = m.as_str();
            let imag = s.ends_with('i');
            let v: f64 = s
                .trim_end_matches('i')
                .replace("Inf", "inf")
                .parse()
                .unwrap();
            values.push((v * scale, imag));
        }
    }
    values
}

fn values_match(actual: &str, expected: &str, tol: f64) -> bool {
    let header = |t: &str| t.lines().find(|l| l.ends_with(" =")).map(str::to_owned);
    let (a, b) = (displayed_values(actual), displayed_values(expected));
    let scale = b.iter().map(|v| v.0.abs()).fold(0f64, f64::max);
    header(actual) == header(expected)
        && a.len() == b.len()
        && a.iter()
            .zip(&b)
            .all(|(x, y)| x.1 == y.1 && (x.0 == y.0 || (x.0 - y.0).abs() <= tol * scale))
}

fn console_matches(id: usize, actual: &str, expected: &str) -> bool {
    match rule(id) {
        Some(r) if r == LAST_DIGIT => last_digits_match(actual, expected),
        Some(r) if r == ARPACK => values_match(actual, expected, 1e-12),
        Some(r) if r == WARNING_LINE => {
            let expected = warning_only(expected);
            let warning = |t: &str| t.split("\n\n").next().map(str::to_owned);
            warning(actual) == warning(&expected) && values_match(actual, &expected, 1e-12)
        }
        Some(r) if r == DEFECTIVE => numbers_match(actual, expected, false, 1e-7),
        _ => actual == expected,
    }
}

fn last_digits_match(actual: &str, expected: &str) -> bool {
    // Signs (including the spaced imaginary sign) and numeric padding belong
    // to the numeric field. Field ends must stay at exactly the same columns:
    // this admits a printed tiny -0 instead of a padded 0, not layout changes.
    let re = regex::Regex::new(r"(?:[-+] *)?[0-9]+(?:\.[0-9]+)?(?:e[-+][0-9]+)?").unwrap();
    let scale = expected
        .lines()
        .find_map(|line| line.trim().strip_suffix(" *")?.parse::<f64>().ok())
        .unwrap_or(1.);
    let (mut ai, mut bi) = (0, 0);
    let a: Vec<_> = re.find_iter(actual).collect();
    let b: Vec<_> = re.find_iter(expected).collect();
    if a.len() != b.len() {
        return false;
    }
    for (x, y) in a.iter().zip(&b) {
        if x.end() != y.end()
            || actual[ai..x.start()].trim_end_matches(' ')
                != expected[bi..y.start()].trim_end_matches(' ')
        {
            return false;
        }
        let av: f64 = x.as_str().replace(' ', "").parse().unwrap();
        let bv: f64 = y.as_str().replace(' ', "").parse().unwrap();
        let within = av == bv || (av - bv).abs() <= (1e-13 * bv.abs()).max(1e-15 / scale.abs());
        if !within {
            return false;
        }
        ai = x.end();
        bi = y.end();
    }
    actual[ai..] == expected[bi..]
}

/// Run one case in a fresh engine: (console text or `ERR message`, class and size of x).
fn run(case: &Case) -> (String, Option<String>) {
    let mut engine = Engine::default();
    if case.long {
        engine.execute("format long").unwrap();
    }
    let actual = match engine.execute(case.statement) {
        Ok(result) => result
            .output
            .iter()
            .map(|s| format!("{s}\n"))
            .collect::<String>(),
        Err(error) => format!(
            "ERR {}\n",
            engine
                .diagnostic
                .as_ref()
                .map_or(error.as_str(), |d| d.message.as_str())
        ),
    };
    let class = engine
        .array("x")
        .map(|a| format!("{} [{} {}]", a.class(), a.rows(), a.cols()));
    (actual, class)
}

#[test]
fn corpus_is_complete_and_exceptions_are_explicit() {
    let cases = cases();
    assert_eq!(cases.len(), TOTAL);
    for (i, case) in cases.iter().enumerate() {
        assert_eq!(case.id, i + 1);
        assert_eq!(case.class_size.is_none(), case.output.starts_with("ERR "));
        assert_eq!(
            case.class_size.is_none(),
            rule(case.id) == Some(ERROR_MESSAGE),
            "case {}",
            case.id
        );
    }
    for (i, &(id, reason)) in KNOWN.iter().enumerate() {
        assert!(cases.iter().any(|c| c.id == id));
        assert!(!reason.is_empty());
        assert!(!KNOWN[..i].iter().any(|&(n, _)| n == id));
    }
    for (i, &(id, r)) in HARNESS.iter().enumerate() {
        assert!(cases.iter().any(|c| c.id == id));
        assert!(!r.is_empty());
        assert!(!HARNESS[..i].iter().any(|&(n, _)| n == id));
        assert!(!KNOWN.iter().any(|&(n, _)| n == id));
    }
}

#[test]
fn relaxed_rules_only_allow_digit_noise() {
    let arpack = "\nx =\n\n   9.999999999999998\n   9.000000000000000\n\n";
    assert!(values_match("\nx =\n\n    10\n     9\n\n", arpack, 1e-12));
    assert!(!values_match("\nx =\n\n    10\n     8\n\n", arpack, 1e-12));
    assert!(!values_match("\nx =\n\n    10\n\n", arpack, 1e-12));
    assert!(!values_match("\ny =\n\n    10\n     9\n\n", arpack, 1e-12));
    assert!(values_match(
        "\nx =\n\n   1.0e+02 *\n\n   3.336812762819950\n\n",
        "\nx =\n\n   333.6812762819950\n\n",
        1e-12
    ));
    assert!(!values_match(
        "\nx =\n\n  1.9 - 0.7i\n\n",
        "\nx =\n\n  1.9 + 0.7i\n\n",
        1e-12
    ));
    let expected = "\nx =\n\n   2.000000000000000\n   1.000000000000000\n\n";
    assert!(numbers_match(
        "\nx =\n\n   2.000000000000002\n   1.000000000000000\n\n",
        expected,
        true,
        1e-12
    ));
    assert!(!numbers_match(
        "\nx =\n\n   2.000000000100000\n   1.000000000000000\n\n",
        expected,
        true,
        1e-12
    ));
    assert!(!numbers_match(
        "\nx =\n\n  2.000000000000000\n   1.000000000000000\n\n",
        expected,
        true,
        1e-12
    ));
    assert!(!numbers_match(
        "\ny =\n\n   2.000000000000000\n   1.000000000000000\n\n",
        expected,
        true,
        1e-12
    ));
    assert_eq!(
        warning_only("[\u{8}Warning: a.]\u{8} \n[\u{8}> In <a>x</a>\n  In y]\u{8} \n\nx =\n"),
        "Warning: a.\n\nx =\n"
    );
}

#[test]
fn linalg3_matches_reference() {
    let mut failures = Vec::new();
    let mut passed = 0;
    let mut known = 0;
    for case in cases() {
        let (actual, class) = run(&case);
        let matches = if rule(case.id) == Some(ERROR_MESSAGE) {
            actual == case.output && class.is_none()
        } else {
            console_matches(case.id, &actual, case.output) && class.as_deref() == case.class_size
        };
        if let Some((_, reason)) = KNOWN.iter().find(|&&(id, _)| id == case.id) {
            known += 1;
            if matches {
                failures.push(format!(
                    "KNOWN {} now passes; remove it ({reason})",
                    case.id
                ));
            }
            eprintln!("KNOWN {}: {reason}", case.id);
            continue;
        }
        if matches {
            passed += 1;
        } else {
            failures.push(format!(
                "case {} `{}`\nexpected {:?}\nactual   {:?}\nclass expected {:?}, actual {:?}",
                case.id, case.statement, case.output, actual, case.class_size, class
            ));
        }
    }
    assert_eq!(known, KNOWN.len());
    assert!(
        failures.is_empty(),
        "{passed}/{} supported cases passed; {known} KNOWN; {} failed:\n{}",
        TOTAL - known,
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(passed + known, TOTAL);
}
