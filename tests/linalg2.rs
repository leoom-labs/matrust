//! lu, qr, chol, pinv, null, orth, expm, logm, sqrtm, mldivide, mrdivide, cond, norm.
//! The review cases use stored reference values; factor signs that LAPACK leaves free
//! (null/orth bases) are checked through invariants (projectors, orthogonality, residuals).
use matrust::{Engine, Matrix};

fn run(src: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(src).unwrap_or_else(|err| panic!("{src}: {err}"));
    e
}
fn fail(src: &str) -> String {
    Engine::default().execute(src).unwrap_err()
}
fn scalar(e: &Engine, name: &str) -> f64 {
    let m: &Matrix = &e.variables[name];
    assert_eq!(m.len(), 1);
    m.data()[0]
}
fn close(src: &str, name: &str, want: f64, tol: f64) {
    let got = scalar(&run(src), name);
    assert!(
        (got - want).abs() <= tol * want.abs().max(1.0),
        "{src}: {got} vs {want}"
    );
}
fn small(src: &str) {
    let e = run(src);
    let got = scalar(&e, "err");
    assert!(got < 1e-12, "{src}: err {got}");
}
fn values(e: &Engine, name: &str, shape: (usize, usize), want: &[f64], tol: f64) {
    let m: &Matrix = &e.variables[name];
    assert_eq!((m.rows(), m.cols()), shape, "{name} shape");
    for (g, w) in m.data().iter().zip(want) {
        assert!(
            (g - w).abs() <= tol * w.abs().max(1.0),
            "{name}: {:?} vs {want:?}",
            m.data()
        );
    }
}

const A: &str = "A=[1 2 3;4 5 6;7 8 10]; B=[2 -1 0;4 3 1]; ";

#[test]
fn lu_forms_and_pivoting() {
    let e = run(&format!(
        "{A}[L,U,P]=lu(A); [L2,U2]=lu(A); Y=lu(A); [~,~,p]=lu(A,'vector');"
    ));
    values(
        &e,
        "L",
        (3, 3),
        &[1., 1. / 7., 4. / 7., 0., 1., 0.5, 0., 0., 1.],
        1e-14,
    );
    values(
        &e,
        "U",
        (3, 3),
        &[7., 0., 0., 8., 6. / 7., 0., 10., 11. / 7., -0.5],
        1e-14,
    );
    values(&e, "P", (3, 3), &[0., 1., 0., 0., 0., 1., 1., 0., 0.], 0.0);
    values(&e, "p", (1, 3), &[3., 1., 2.], 0.0);
    values(
        &e,
        "Y",
        (3, 3),
        &[7., 1. / 7., 4. / 7., 8., 6. / 7., 0.5, 10., 11. / 7., -0.5],
        1e-14,
    );
    small(&format!("{A}[L,U]=lu(A); err=norm(L*U-A,1);"));
    small(&format!("{A}[L,U,P]=lu(B'); err=norm(L*U-P*B',1);"));
    small(&format!(
        "{A}[L,U,P]=lu(B); err=norm(L*U-P*B,1); [r,c]=size(L); if r~=2||c~=2,err=1;end"
    ));
    let e = run("[L,U,P]=lu(zeros(0,3)); a=size(U,2);");
    assert_eq!(scalar(&e, "a"), 3.0);
}

#[test]
fn qr_signs_economy_and_pivoting() {
    let e = run("[Q,R]=qr([1 2;3 4]);");
    let s = 10f64.sqrt();
    values(&e, "Q", (2, 2), &[-1. / s, -3. / s, -3. / s, 1. / s], 1e-14);
    values(&e, "R", (2, 2), &[-s, 0., -14. / s, -2. / s], 1e-14);
    small(&format!(
        "{A}[Q,R]=qr(A); err=norm(Q*R-A,1)+norm(Q'*Q-eye(3),1);"
    ));
    small(&format!(
        "{A}[Q,R]=qr(B'); err=norm(Q*R-B',1)+norm(Q'*Q-eye(3),1);"
    ));
    small(&format!(
        "{A}[Q,R]=qr(B); err=norm(Q*R-B,1)+norm(Q'*Q-eye(2),1);"
    ));
    let e = run(&format!(
        "{A}[Q,R]=qr(B',0); a=size(Q,2); b=size(R,1); c=norm(Q*R-B',1);"
    ));
    assert_eq!((scalar(&e, "a"), scalar(&e, "b")), (2.0, 2.0));
    assert!(scalar(&e, "c") < 1e-13);
    small(&format!(
        "{A}[Q,R,E]=qr(A); err=norm(Q*R-A*E,1); d=abs(diag(R)); if d(1)<d(2)||d(2)<d(3),err=1;end"
    ));
    small(&format!(
        "{A}[Q,R,p]=qr(B',0); T=B'; err=norm(Q*R-T(:,p),1);"
    ));
    // A single output is the full-size packed factorization.
    let e = run(&format!("{A}R=qr(B'); [r,c]=size(R);"));
    assert_eq!((scalar(&e, "r"), scalar(&e, "c")), (3.0, 2.0));
}

#[test]
fn qr_single_output_is_packed_even_in_economy_form() {
    let s = 10f64.sqrt();
    for option in ["", ",0"] {
        let e = run(&format!("X=qr([1 2;3 4]{option});"));
        values(
            &e,
            "X",
            (2, 2),
            &[-s, 3. / (1. + s), -14. / s, -2. / s],
            1e-14,
        );
        let e = run(&format!("{A}X=qr(B'{option});"));
        values(
            &e,
            "X",
            (3, 2),
            &[
                -5f64.sqrt(),
                -0.236067977499790,
                0.,
                -5f64.sqrt(),
                -21f64.sqrt(),
                0.110439739956261,
            ],
            1e-14,
        );
        let e = run(&format!("X=qr(zeros(3,0){option});"));
        values(&e, "X", (3, 0), &[], 0.0);
    }
}

#[test]
fn lu_matrix_option_matches_default_for_all_output_counts() {
    for call in [
        "Y=lu(A,'matrix'); Y0=lu(A); err=norm(Y-Y0,1);",
        "[L,U]=lu(A,'matrix'); err=norm(L*U-A,1);",
        "[L,U,P]=lu(A,'matrix'); [L0,U0,P0]=lu(A); err=norm(L-L0,1)+norm(U-U0,1)+norm(P-P0,1);",
    ] {
        small(&format!("{A}{call}"));
    }
}

#[test]
fn qr_explicit_permutation_forms() {
    let e = run(&format!(
        "{A}[Q,R,p]=qr(A,'vector'); [Qm,Rm,E]=qr(A,'matrix');"
    ));
    values(&e, "p", (1, 3), &[3., 1., 2.], 0.0);
    values(&e, "E", (3, 3), &[0., 0., 1., 1., 0., 0., 0., 1., 0.], 0.0);
    small(&format!(
        "{A}[Q,R,p]=qr(A,'vector'); err=norm(Q*R-A(:,p),1)+norm(Q'*Q-eye(3),1);"
    ));
    small(&format!(
        "{A}[Q,R,E]=qr(A,'matrix'); err=norm(Q*R-A*E,1)+norm(Q'*Q-eye(3),1);"
    ));
    for option in ["vector", "matrix"] {
        let e = run(&format!("{A}[Q,R,E]=qr(B','{option}');"));
        assert_eq!((e.variables["Q"].rows(), e.variables["Q"].cols()), (3, 3));
        assert_eq!((e.variables["R"].rows(), e.variables["R"].cols()), (3, 2));
    }
    assert!(fail("[Q,R,E]=qr(ones(1,100000),'matrix');").contains("element limit"));
    let e = run("[Q,R,p]=qr(ones(1,100000),'vector'); n=numel(p);");
    assert_eq!(scalar(&e, "n"), 100000.0);
}

#[test]
fn chol_forms() {
    let e = run(
        "R=chol([4 2;2 3]); L=chol([4 2;2 3],'lower'); [Rp,p]=chol([1 2;2 1]); [~,q]=chol([4 2;2 3]);",
    );
    values(&e, "R", (2, 2), &[2., 0., 1., 2f64.sqrt()], 1e-14);
    values(&e, "L", (2, 2), &[2., 1., 0., 2f64.sqrt()], 1e-14);
    values(&e, "Rp", (1, 1), &[1.], 0.0);
    assert_eq!(scalar(&e, "p"), 2.0);
    assert_eq!(scalar(&e, "q"), 0.0);
    // only the upper triangle is read
    let e = run("R=chol([4 2;9 3]);");
    values(&e, "R", (2, 2), &[2., 0., 1., 2f64.sqrt()], 1e-14);
    assert!(fail("chol([1 2;2 1])").ends_with("Matrix must be positive definite."));
    small(
        "M=[8 1 6 3 5 7;3 5 7 9 1 2;4 9 2 6 8 1;1 2 3 4 5 6;9 8 7 6 5 4;2 7 1 8 3 9]; M=M*M'+6*eye(6); R=chol(M); err=norm(R'*R-M,1);",
    );
}

#[test]
fn svd_based_functions() {
    let e = run(&format!(
        "{A}P=pinv(B); Q=pinv([1 2;2 4]); Z=pinv(zeros(2,3)); T=pinv([1 2;2 4],10);"
    ));
    small(&format!(
        "{A}P=pinv(B); err=norm(B*P*B-B,1)+norm(P*B*P-P,1)+norm((B*P)'-B*P,1);"
    ));
    values(&e, "Q", (2, 2), &[0.04, 0.08, 0.08, 0.16], 1e-13);
    values(&e, "Z", (3, 2), &[0.; 6], 0.0);
    values(&e, "T", (2, 2), &[0.; 4], 0.0);
    // null/orth: signs are free, so test spans
    small(&format!(
        "{A}N=null(B); err=norm(B*N,1)+norm(N'*N-eye(1),1); if size(N,2)~=1,err=1;end"
    ));
    small(&format!(
        "{A}N=null([1 1 1]); err=norm([1 1 1]*N,1)+norm(N'*N-eye(2),1);"
    ));
    small(&format!(
        "{A}N=null([1 2;2 4]); err=norm(N-[-2;1]/sqrt(5),1); if err>1e-12,err=norm(N-[2;-1]/sqrt(5),1);end"
    ));
    small(&format!(
        "{A}O=orth([1 2;2 4]); err=norm(O*O'*[1;2]-[1;2],1)+norm(O'*O-1,1);"
    ));
    small(&format!(
        "{A}O=orth(B); err=norm(O'*O-eye(2),1)+norm(O*O'*B-B,1);"
    ));
    let e = run(&format!(
        "{A}a=size(null(A),2); b=size(orth(zeros(2,3)),2); c=size(null(zeros(2,3)),2); d=size(null([]),1);"
    ));
    assert_eq!(
        (
            scalar(&e, "a"),
            scalar(&e, "b"),
            scalar(&e, "c"),
            scalar(&e, "d")
        ),
        (0.0, 0.0, 3.0, 0.0)
    );
    close(&format!("{A}c=cond(A);"), "c", 88.44827973, 1e-8);
    close(&format!("{A}c=cond(A,1);"), "c", 133.0, 1e-12);
    close(&format!("{A}c=cond(A,Inf);"), "c", 158.3333333333333, 1e-12);
    close(&format!("{A}c=cond(B);"), "c", 2.647588, 1e-6);
    assert!(fail("cond([1 2 3;4 5 6],1)").contains("rectangular"));
    assert!(fail("pinv([1 NaN;2 4])").contains("must not contain NaN or Inf"));
}

#[test]
fn norms() {
    close("n=norm([3 4]);", "n", 5.0, 1e-15);
    close("n=norm([3 4]',1);", "n", 7.0, 1e-15);
    close("n=norm([3 4],Inf);", "n", 4.0, 0.0);
    close("n=norm([3 4],-Inf);", "n", 3.0, 0.0);
    close("n=norm([3 4],'inf');", "n", 4.0, 0.0);
    close("n=norm([3 4],3);", "n", 91f64.powf(1.0 / 3.0), 1e-14);
    // Reference behavior: any real p is sum(abs(v).^p)^(1/p); p = 0 gives Inf.
    close("n=norm([3 4],-1);", "n", 12.0 / 7.0, 1e-15);
    close("n=norm([3 4],-2);", "n", 2.4, 1e-15);
    assert_eq!(scalar(&run("n=norm([3 4],0);"), "n"), f64::INFINITY);
    close("n=norm([3 4],'fro');", "n", 5.0, 1e-15);
    close("n=norm([]);", "n", 0.0, 0.0);
    close(&format!("{A}n=norm(A,1);"), "n", 19.0, 0.0);
    close(&format!("{A}n=norm(A,Inf);"), "n", 25.0, 0.0);
    close(&format!("{A}n=norm(A,'fro');"), "n", 304f64.sqrt(), 1e-14);
    close(&format!("{A}n=norm(A);"), "n", 17.4125052, 1e-8);
    assert!(scalar(&run("n=norm([1 NaN;2 3]);"), "n").is_nan());
    assert_eq!(scalar(&run("n=norm([1 Inf;2 3]);"), "n"), f64::INFINITY);
    assert!(fail("norm(ones(2),3)").contains("The only matrix norms available"));
    assert!(fail("norm(1,2,3)").ends_with("Too many input arguments."));
    close("n=norm(true);", "n", 1.0, 0.0);
}

#[test]
fn matrix_functions() {
    // MATLAB documentation values for A = [1 2;3 4]
    let e = run("E=expm([1 2;3 4]); L=logm([1 2;3 4]); S=sqrtm([4 1;0 9]);");
    values(
        &e,
        "E",
        (2, 2),
        &[51.968956198705, 112.104847, 74.736564567003, 164.073803],
        1e-8,
    );
    values(&e, "S", (2, 2), &[2., 0., 0.2, 3.], 1e-14);
    let l = e.complex_variables["L"].clone();
    let want = [
        (-0.3504398139985536, 2.391117954451724),
        (1.3940268085570522, -1.6406432553136552),
        (0.9293512057047018, -1.0937621702091036),
        (1.0435869945584986, 0.7504746991380685),
    ];
    for (z, (re, im)) in l.data().iter().zip(want) {
        assert!(
            (z.re - re).abs() < 1e-12 && (z.im - im).abs() < 1e-12,
            "{z}"
        );
    }
    small("X=[0 1;-1 0]; err=norm(expm(X)-[cos(1) sin(1);-sin(1) cos(1)],1);");
    small("err=norm(expm(zeros(3))-eye(3),1);");
    small("M=[1 2 3;4 5 6;7 8 10]/10; err=norm(expm(M)*expm(-M)-eye(3),1);");
    small("M=[1 2 3;4 5 6;7 8 10]*3; err=norm(expm(M)*expm(-M)-eye(3),1)/norm(expm(M),1);");
    small("err=abs(expm(1)-exp(1))+norm(expm([])-[],1);");
    small("M=[2 1;1 2]; X=sqrtm(M); err=norm(X*X-M,1);");
    small("M=[4 1 0;1 5 2;0 2 6]; X=sqrtm(M); err=norm(X*X-M,1);");
    let e = run("X=sqrtm([1 2;3 4]);");
    let x = &e.complex_variables["X"];
    let (a, b, c, d) = (x.data()[0], x.data()[1], x.data()[2], x.data()[3]);
    for (got, want) in [
        (a * a + c * b, 1.0),
        (a * c + c * d, 2.0),
        (b * a + d * b, 3.0),
        (b * c + d * d, 4.0),
    ] {
        assert!(
            (got.re - want).abs() < 1e-13 && got.im.abs() < 1e-13,
            "{got}"
        );
    }
    small("M=[4 1 0;1 5 2;0 2 6]; err=norm(expm(logm(M))-M,1)/norm(M,1);");
    small("M=[1 2 3;0 4 5;0 0 6]; err=norm(expm(logm(M))-M,1)/norm(M,1);");
    small("err=norm(logm(eye(3)),1);");
    close("a=real(sqrtm(-4)); b=imag(sqrtm(-4));", "b", 2.0, 1e-14);
    close("b=imag(logm(-1));", "b", std::f64::consts::PI, 1e-14);
    assert!(fail("expm([1 2 3])").ends_with("Matrix must be square."));
    assert!(fail("logm(0)").contains("singular"));
}

#[test]
fn divide_functions_and_arity() {
    small(&format!(
        "{A}b=[1;2;3]; err=norm(mldivide(A,b)-A\\b,1)+norm(mrdivide(b',A)-b'/A,1);"
    ));
    small(&format!("{A}b=[1;2;3]; err=norm(A*mldivide(A,b)-b,1);"));
    close("x=mrdivide(4,2);", "x", 2.0, 0.0);
    assert!(fail("lu()").ends_with("Not enough input arguments."));
    assert!(fail("[a,b,c,d]=lu(1)").ends_with("Too many output arguments."));
    assert!(fail("lu('a')").contains("char"));
}

#[test]
fn output_sizes_are_checked_before_allocation() {
    // Full Q (m-by-m), P (m-by-m) and E (n-by-n) of a long vector exceed the element limit.
    for src in [
        "[Q,R]=qr(ones(100000,1));",
        "[L,U,P]=lu(ones(100000,1));",
        "[Q,R,E]=qr(ones(1,100000));",
    ] {
        assert!(fail(src).contains("element limit"), "{src}");
    }
    // Wide null/orth pad to n-by-n: rejected before the padded copy exists.
    assert!(fail("N=null(ones(1,100000));").contains("limit"));
    assert!(fail("O=orth(ones(1,100000));").contains("limit"));
    // The same inputs work where the outputs stay small.
    let e = run(
        "R=qr(ones(100000,1)); [Q,R2]=qr(ones(100000,1),0); [L,U,p]=lu(ones(100000,1),'vector'); a=size(Q,2); b=numel(p);",
    );
    assert_eq!((scalar(&e, "a"), scalar(&e, "b")), (1.0, 100000.0));
    assert!((scalar(&e, "R2") + 100000f64.sqrt()).abs() < 1e-9);
}

#[test]
fn sqrtm_of_singular_triangular_matrices() {
    let e = run("X=sqrtm(zeros(2)); Y=sqrtm([0 1;0 0]); Z=sqrtm([4 0 0;0 0 0;0 0 9]);");
    values(&e, "X", (2, 2), &[0.; 4], 0.0);
    let y: &Matrix = &e.variables["Y"];
    assert_eq!(y.data(), &[0.0, 0.0, f64::INFINITY, 0.0]);
    values(&e, "Z", (3, 3), &[2., 0., 0., 0., 0., 0., 0., 0., 3.], 0.0);
    let e = run("W=sqrtm([0 0 1;0 0 0;0 0 0]);");
    let w: &Matrix = &e.variables["W"];
    for j in 0..3 {
        for i in 0..3 {
            if i < j {
                assert!(w.get(i, j).is_nan(), "W({i},{j}) = {}", w.get(i, j));
            } else {
                assert_eq!(w.get(i, j), 0.0);
            }
        }
    }
}

#[test]
fn qr_reflector_does_not_overflow_or_underflow() {
    let e = run("[Q,R]=qr([1e308;1e307]); [Q2,R2]=qr([1e-310;1e-310]);");
    for name in ["Q", "R", "Q2", "R2"] {
        let m: &Matrix = &e.variables[name];
        assert!(
            m.data().iter().all(|v| v.is_finite()),
            "{name}: {:?}",
            m.data()
        );
    }
    let r = scalar_at(&e, "R", 0);
    assert!((r + 1e308f64.hypot(1e307)).abs() <= 1e-15 * r.abs(), "{r}");
    let r2 = scalar_at(&e, "R2", 0);
    assert!(
        (r2 + 2f64.sqrt() * 1e-310).abs() <= 1e-12 * r2.abs(),
        "{r2}"
    );
    small("A=[1e308;1e307]; [Q,R]=qr(A); err=norm(Q*R-A,1)/norm(A,1)+norm(Q'*Q-eye(2),1);");
    small("A=[1e-310;1e-310]; [Q,R]=qr(A); err=norm(Q*R-A,1)/norm(A,1)+norm(Q'*Q-eye(2),1);");
    small("A=[1e308;1e307]; [Q,R,E]=qr(A); err=norm(Q*R-A,1)/norm(A,1)+norm(Q'*Q-eye(2),1);");
}

#[test]
fn infinity_norms_keep_nan() {
    for src in [
        "n=norm([NaN 1],Inf);",
        "n=norm([1 NaN],Inf);",
        "n=norm([NaN 1],-Inf);",
        "n=norm([NaN;1],'inf');",
    ] {
        assert!(scalar(&run(src), "n").is_nan(), "{src}");
    }
}

fn scalar_at(e: &Engine, name: &str, i: usize) -> f64 {
    e.variables[name].data()[i]
}

#[test]
fn large_inputs_are_not_double_charged() {
    let e = run("A=zeros(3000); n=norm(A,1);");
    assert_eq!(scalar(&e, "n"), 0.0);
    assert!(fail("[Q,R]=qr(ones(100000,1));").contains("element limit"));
}

#[test]
fn reference_reviewer_cases() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("linalg2-expected-rev3.json")).unwrap();
    assert_eq!(cases.len(), 85);
    for case in cases {
        let source = case["source"].as_str().unwrap();
        let mut e = run(A);
        let result = e.execute(&format!("{source};"));
        let mut actual = serde_json::json!({"source": source, "values": {}});
        if let Err(error) = &result {
            actual["error"] = error.clone().into();
        } else {
            for name in ["X", "L", "U", "P", "Q", "R", "E", "p"] {
                if let Some(m) = e.variables.get(name) {
                    actual["values"][name] = serde_json::json!({
                        "rows": m.rows(), "cols": m.cols(),
                        "data": m.data().iter().map(f64::to_string).collect::<Vec<_>>()
                    });
                } else if let Some(m) = e.complex_variables.get(name) {
                    actual["values"][name] = serde_json::json!({
                        "rows": m.rows(), "cols": m.cols(),
                        "data": m.data().iter().map(|v| v.re.to_string()).collect::<Vec<_>>(),
                        "imag": m.data().iter().map(|v| v.im.to_string()).collect::<Vec<_>>()
                    });
                }
            }
        }
        println!("LA2_ORACLE {actual}");
        // Keep documented differences visible without pinning those limitations in tests.
        if !case["identical_values"].as_bool().unwrap() {
            continue;
        }
        if let Some(error) = case["error"].as_str() {
            assert!(result.unwrap_err().ends_with(error), "{source}");
            continue;
        }
        result.unwrap_or_else(|err| panic!("{source}: {err}"));
        for (name, expected) in case["values"].as_object().unwrap() {
            let m = &e.variables[name];
            assert_eq!(
                m.rows(),
                expected["rows"].as_u64().unwrap() as usize,
                "{source}"
            );
            assert_eq!(
                m.cols(),
                expected["cols"].as_u64().unwrap() as usize,
                "{source}"
            );
            let data = expected["data"].as_array().unwrap();
            assert_eq!(m.len(), data.len());
            // LAPACK leaves basis signs/rotations free. Compare the projectors,
            // even for a basis that happened to match the recorded run exactly.
            if source.contains("=null(") || source.contains("=orth(") {
                let want: Vec<f64> = data
                    .iter()
                    .map(|v| v.as_str().unwrap().parse().unwrap())
                    .collect();
                for j in 0..m.rows() {
                    for i in 0..m.rows() {
                        let got: f64 = (0..m.cols()).map(|k| m.get(i, k) * m.get(j, k)).sum();
                        let want: f64 = (0..m.cols())
                            .map(|k| want[k * m.rows() + i] * want[k * m.rows() + j])
                            .sum();
                        assert!((got - want).abs() < 1e-12, "{source}: basis projector");
                    }
                }
                continue;
            }
            for (&got, want) in m.data().iter().zip(data) {
                let want: f64 = want.as_str().unwrap().parse().unwrap();
                assert!(
                    got == want
                        || (got.is_nan() && want.is_nan())
                        || (got.is_finite()
                            && want.is_finite()
                            && (got - want).abs() <= 1e-12 * want.abs().max(1.0)),
                    "{source}, {name}: {got} vs {want}"
                );
            }
        }
    }
}

#[test]
#[ignore = "release measurement; run explicitly with LA2_BENCH_WORKERS=1 and 4"]
fn benchmark_round3_paths() {
    let workers = std::env::var("LA2_BENCH_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse::<usize>()
        .unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    for (name, rows, cols, repetitions, setup, operation) in [
        ("packed QR small", 2, 2, 1000, "A=[1 2;3 4];", "X=qr(A,0);"),
        ("packed QR tall", 256, 64, 40, "", "X=qr(A,0);"),
        ("sqrtm diagonal", 64, 64, 200, "A=4*eye(64);", "X=sqrtm(A);"),
        (
            "sqrtm triangular",
            64,
            64,
            100,
            "A=4*eye(64)+diag(ones(63,1),1);",
            "X=sqrtm(A);",
        ),
    ] {
        let mut engine = Engine::default();
        if setup.is_empty() {
            let data = (0..rows * cols)
                .map(|i| ((i * 17 + 3) as f64).sin())
                .collect();
            engine.variables.insert(
                "A".into(),
                Matrix::from_column_vec(rows, cols, data).unwrap(),
            );
        } else {
            engine.execute(setup).unwrap();
        }
        for _ in 0..3 {
            engine.execute(operation).unwrap();
        }
        let start = std::time::Instant::now();
        for _ in 0..repetitions {
            engine.execute(operation).unwrap();
            std::hint::black_box(&engine.variables["X"]);
        }
        println!(
            "LA2_BENCH workers={workers} path={name:?} size={rows}x{cols} repetitions={repetitions} us_per_call={:.3}",
            start.elapsed().as_secs_f64() * 1e6 / repetitions as f64
        );
    }
}
