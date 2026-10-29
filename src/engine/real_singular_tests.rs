//! Regression evidence: reference warning wording supplied in the
//! task; result values use the existing complex template and Matrix kernels.
//! No live MATLAB reference is invoked by these tests.
use super::Engine;
use crate::Matrix;

const SINGULAR: &str = "Warning: Matrix is singular to working precision.";
// Unchanged dlacn2 + nalgebra LU: the reference output supplied in the task is 4.625929e-18.
const CLOSE: &str = "Warning: Matrix is close to singular or badly scaled. Results may be inaccurate. RCOND = 1.306145e-17.";
// Right division factors A transpose, whose LU roundoff differs.
const CLOSE_TRANSPOSE: &str = "Warning: Matrix is close to singular or badly scaled. Results may be inaccurate. RCOND = 9.796086e-18.";
const NONFINITE: &str = "Warning: Matrix is singular, close to singular or badly scaled. Results may be inaccurate. RCOND = NaN.";

fn same_values(actual: &Matrix, expected: &Matrix) {
    assert_eq!(
        (actual.rows(), actual.cols()),
        (expected.rows(), expected.cols())
    );
    for (&a, &b) in actual.data().iter().zip(expected.data()) {
        assert!(
            (a.is_nan() && b.is_nan())
                || a == b
                || (a.is_finite() && b.is_finite() && (a - b).abs() <= 1e-14 * b.abs().max(1.)),
            "{a:?} != {b:?}"
        );
    }
}

#[test]
fn real_singular_magic_solve_inverse_power_and_right_division() {
    let mut engine = Engine::default();
    engine
        .execute("A=magic(4);b=[1;2;3;4];r=[1 2 3 4];")
        .unwrap();
    let a = engine.variables["A"].clone();
    let b = engine.variables["b"].clone();
    let r = engine.variables["r"].clone();
    // Retain the pre-change numerical results, including LU roundoff in this
    // mathematically singular matrix, while adding the RCOND diagnostic.
    let inverse = a.inverse().unwrap();
    let mut actual_output = Vec::new();
    let mut expected_output = Vec::new();
    for (code, expected) in [
        ("x=A\\b;", a.solve(&b).unwrap()),
        ("x=mldivide(A,b);", a.solve(&b).unwrap()),
        ("x=inv(A);", inverse.clone()),
        ("x=A^-1;", a.power(-1.).unwrap()),
        ("x=A^-2;", inverse.power(2.).unwrap()),
        (
            "x=r/A;",
            a.transpose().solve(&r.transpose()).unwrap().transpose(),
        ),
        (
            "x=mrdivide(r,A);",
            a.transpose().solve(&r.transpose()).unwrap().transpose(),
        ),
    ] {
        let out = engine.execute(code.trim_end_matches(';')).unwrap();
        actual_output.push((code, out.output));
        let warning = if matches!(code, "x=r/A;" | "x=mrdivide(r,A);") {
            CLOSE_TRANSPOSE
        } else {
            CLOSE
        };
        expected_output.push((
            code,
            vec![
                warning.to_owned(),
                crate::display_format::echo_real("x", &expected, &engine.display).unwrap(),
            ],
        ));
        same_values(&engine.variables["x"], &expected);
    }
    assert_eq!(actual_output, expected_output);
}

#[test]
fn real_singular_zero_solve_and_inverse_match_complex_template() {
    let mut engine = Engine::default();
    for (real, complex) in [
        ("zeros(2)\\[1;2]", "complex(zeros(2))\\[1;2]"),
        ("[1 2]/zeros(2)", "[1 2]/complex(zeros(2))"),
        ("inv(zeros(2))", "inv(complex(zeros(2)))"),
        // Exercise an exactly singular nontriangular LU factor as well.
        ("[1 2;2 4]\\[1;2]", "complex([1 2;2 4])\\[1;2]"),
    ] {
        let out = engine.execute(&format!("x={real};")).unwrap();
        assert_eq!(out.output, [SINGULAR], "{real}");
        let x = engine.variables["x"].clone();
        let out = engine.execute(&format!("z={complex};")).unwrap();
        assert_eq!(out.output, [SINGULAR], "{complex}");
        let z = if let Some(z) = engine.variables.get("z") {
            z.clone()
        } else {
            engine.complex_variables["z"].real(&engine.cancel).unwrap()
        };
        same_values(&x, &z);
    }
    let out = engine.execute("x=zeros(2)\\[1;2];").unwrap();
    assert_eq!(out.output, [SINGULAR]);
    assert_eq!(engine.variables["x"].data()[0], f64::INFINITY);
    assert!(engine.variables["x"].data()[1].is_nan());
    let out = engine.execute("x=inv(zeros(2));").unwrap();
    assert_eq!(out.output, [SINGULAR]);
    assert_eq!(engine.variables["x"].data(), &[f64::INFINITY; 4]);
    for code in ["x=mldivide(zeros(2),[1;2]);", "x=mrdivide([1 2],zeros(2));"] {
        assert_eq!(engine.execute(code).unwrap().output, [SINGULAR]);
    }
    // Internal numerical callers retain their explicit singular errors.
    let zero = Matrix::filled(2, 2, 0.).unwrap();
    assert!(zero.inverse().is_err());
    assert!(zero.solve(&Matrix::identity(2, 2).unwrap()).is_err());
}

#[test]
fn real_singular_well_conditioned_scalar_and_rectangular_paths() {
    let mut engine = Engine::default();
    for code in ["x=[2 1;1 3]\\[1;2];", "x=mldivide([2 1;1 3],[1;2]);"] {
        assert!(engine.execute(code).unwrap().output.is_empty());
        same_values(
            &engine.variables["x"],
            &Matrix::from_column_vec(2, 1, vec![0.2, 0.6]).unwrap(),
        );
    }
    assert!(engine.execute("x=inv(eye(3));").unwrap().output.is_empty());
    same_values(&engine.variables["x"], &Matrix::identity(3, 3).unwrap());
    for code in [
        "x=0\\[1;2];",
        "x=[1 2]/0;",
        "x=mldivide(0,[1;2]);",
        "x=mrdivide([1 2],0);",
    ] {
        assert!(engine.execute(code).unwrap().output.is_empty(), "{code}");
        assert_eq!(engine.variables["x"].data(), &[f64::INFINITY; 2]);
    }
    assert!(engine.execute("x=[1 2 3;4 5 6]\\[1;2];").is_err());
}

#[test]
fn real_singular_nonfinite_warning_and_suppression() {
    let mut engine = Engine::default();
    for code in [
        "x=[NaN 1;1 2]\\[1;2];",
        "x=inv([Inf 0;0 1]);",
        "x=[NaN 0;0 1]\\[1;2];",
    ] {
        assert_eq!(engine.execute(code).unwrap().output, [NONFINITE], "{code}");
    }
    engine.execute("warning('off','all');").unwrap();
    for code in [
        "x=magic(4)\\[1;2;3;4];",
        "x=inv(magic(4));",
        "x=magic(4)^-1;",
        "x=[1 2 3 4]/magic(4);",
        "x=zeros(2)\\[1;2];",
        "x=inv(zeros(2));",
        "x=mldivide(magic(4),[1;2;3;4]);",
        "x=mrdivide([1 2],zeros(2));",
        "x=inv([NaN 0;0 1]);",
    ] {
        assert!(engine.execute(code).unwrap().output.is_empty(), "{code}");
    }
    engine.execute("warning('on','all');").unwrap();
    assert_eq!(
        engine.execute("x=inv(zeros(2));").unwrap().output,
        [SINGULAR]
    );
}

#[test]
fn real_singular_large_well_conditioned_solves_have_no_size_cap() {
    // Above the 1626 dense work cap the first version added; these solved before.
    let mut engine = Engine::default();
    let out = engine.execute("x=(eye(1700)*2)\\ones(1700,1);").unwrap();
    assert!(out.output.is_empty());
    assert_eq!(engine.variables["x"].data(), &[0.5; 1700]);
    // A nontriangular A takes the LU path: the corner block [2 1;1 2] gives
    // x(1) = x(end) = 1/3. inv at this size takes ~30 s, so it is left out.
    let out = engine
        .execute("A=eye(1700)*2; A(1,1700)=1; A(1700,1)=1; x=A\\ones(1700,1);")
        .unwrap();
    assert!(out.output.is_empty());
    let x = engine.variables["x"].data();
    for v in [x[0], x[1699]] {
        assert!((v - 1. / 3.).abs() <= 1e-15, "{v}");
    }
    assert!(x[1..1699].iter().all(|&v| v == 0.5));
}

#[test]
fn real_singular_negative_powers_start_from_the_inverse() {
    let mut engine = Engine::default();
    for code in ["x=zeros(2)^-1;", "x=zeros(2)^-2;", "x=mpower(zeros(2),-1);"] {
        assert_eq!(engine.execute(code).unwrap().output, [SINGULAR], "{code}");
        assert_eq!(engine.variables["x"].data(), &[f64::INFINITY; 4], "{code}");
    }
    engine.execute("A=magic(4);").unwrap();
    let inverse = engine.variables["A"].inverse().unwrap();
    for (code, k) in [("x=magic(4)^-2;", 2.), ("x=A^-3;", 3.)] {
        assert_eq!(engine.execute(code).unwrap().output, [CLOSE], "{code}");
        same_values(&engine.variables["x"], &inverse.power(k).unwrap());
    }
    assert!(engine.execute("x=[2 1;1 3]^-1;").unwrap().output.is_empty());
    same_values(
        &engine.variables["x"],
        &Matrix::from_column_vec(2, 2, vec![0.6, -0.2, -0.2, 0.4]).unwrap(),
    );
}

fn last_warning(engine: &mut Engine, code: &str) -> (Vec<String>, String, String) {
    let output = engine
        .execute(&format!("lastwarn(''); {code} [lw_m,lw_id]=lastwarn;"))
        .unwrap()
        .output;
    let text = |name: &str| match engine.array(name) {
        Some(crate::Array::Character(c)) => String::from_utf16(c.units()).unwrap(),
        _ => panic!("{name} is not char"),
    };
    (output, text("lw_m"), text("lw_id"))
}

#[test]
fn real_singular_warning_identifiers_and_lastwarn() {
    let mut engine = Engine::default();
    let singular = ("MATLAB:singularMatrix", SINGULAR);
    let close = ("MATLAB:nearlySingularMatrix", CLOSE);
    let nonfinite = ("MATLAB:illConditionedMatrix", NONFINITE);
    let complex_close =
        "Warning: Matrix is close to singular or badly scaled. Results may be inaccurate. RCOND = ";
    let cases = [
        ("x=inv(zeros(2));", singular),
        ("x=zeros(2)\\[1;2];", singular),
        ("x=inv(complex(zeros(2)));", singular),
        ("x=complex([1 2;2 4])\\[1;2];", singular),
        ("x=magic(4)\\[1;2;3;4];", close),
        (
            "x=complex(magic(4))\\[1;2;3;4];",
            ("MATLAB:nearlySingularMatrix", ""),
        ),
        ("x=[NaN 1;1 2]\\[1;2];", nonfinite),
        ("x=complex([NaN 0;0 1])\\[1;2];", nonfinite),
    ];
    for (code, (id, warning)) in cases {
        let (output, message, last_id) = last_warning(&mut engine, code);
        assert_eq!(output.len(), 1, "{code}");
        if warning.is_empty() {
            assert!(output[0].starts_with(complex_close), "{code}");
        } else {
            assert_eq!(output[0], warning, "{code}");
        }
        assert_eq!(format!("Warning: {message}"), output[0], "{code}");
        assert_eq!(last_id, id, "{code}");
    }
    // Each identifier silences only its own warning; lastwarn is still set,
    // as the reference sets it for a disabled warning (tests/fixtures lang2 case 79).
    engine
        .execute("warning('off','MATLAB:singularMatrix');")
        .unwrap();
    for (code, (id, _)) in &cases[..4] {
        let (output, _, last_id) = last_warning(&mut engine, code);
        assert!(output.is_empty(), "{code}");
        assert_eq!(&last_id, id, "{code}");
    }
    for (code, _) in &cases[4..] {
        assert_eq!(last_warning(&mut engine, code).0.len(), 1, "{code}");
    }
    engine
        .execute("warning('off','MATLAB:nearlySingularMatrix'); warning('off','MATLAB:illConditionedMatrix');")
        .unwrap();
    for (code, (id, _)) in cases {
        let (output, _, last_id) = last_warning(&mut engine, code);
        assert!(output.is_empty(), "{code}");
        assert_eq!(last_id, id, "{code}");
    }
    engine
        .execute("warning('on','MATLAB:nearlySingularMatrix');")
        .unwrap();
    assert_eq!(last_warning(&mut engine, cases[4].0).0, [CLOSE]);
    // A well-conditioned solve leaves lastwarn alone.
    let (output, message, last_id) = last_warning(&mut engine, "x=[2 1;1 3]\\[1;2];");
    assert!(output.is_empty() && message.is_empty() && last_id.is_empty());
}

/// Focused release timing, opt-in and run in a fresh process per worker count:
/// MATRUST_SINGULAR_WORKERS=1 cargo test --release --lib real_singular_benchmark -- --ignored --nocapture
#[test]
#[ignore = "focused numerical-path benchmark; run explicitly in release mode"]
fn real_singular_benchmark() {
    use std::{hint::black_box, time::Instant};
    let workers = std::env::var("MATRUST_SINGULAR_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    crate::initialize_worker_pool(workers).unwrap();
    let mut engine = Engine::default();
    let mut out = super::Execution::default();
    for (n, repetitions) in [(4, 1000), (64, 100), (256, 10)] {
        let a = Matrix::from_column_vec(
            n,
            n,
            (0..n * n)
                .map(|k| {
                    let (i, j) = (k % n, k / n);
                    if i == j {
                        n as f64 + 1.
                    } else {
                        ((i * 7 + j * 11) % 17) as f64 / 17.
                    }
                })
                .collect(),
        )
        .unwrap();
        let b = Matrix::filled(n, 8, 1.).unwrap();
        for inverse in [false, true] {
            let run_old = || {
                if inverse {
                    a.inverse().unwrap()
                } else {
                    a.solve(&b).unwrap()
                }
            };
            let expected = run_old();
            let mut run_new = || {
                if inverse {
                    engine.real_inverse(&a, &mut out).unwrap()
                } else {
                    engine.real_divide(true, &a, &b, &mut out).unwrap()
                }
            };
            same_values(&run_new(), &expected);
            let start = Instant::now();
            for _ in 0..repetitions {
                black_box(run_old());
            }
            let old = start.elapsed();
            let start = Instant::now();
            for _ in 0..repetitions {
                black_box(run_new());
            }
            let new = start.elapsed();
            assert!(out.output.is_empty());
            println!(
                "workers={workers} n={n} rhs={} repetitions={repetitions} old_us={:.3} new_us={:.3}",
                if inverse { n } else { 8 },
                old.as_secs_f64() * 1e6 / repetitions as f64,
                new.as_secs_f64() * 1e6 / repetitions as f64
            );
        }
    }
}

#[test]
fn real_singular_typed_paths_keep_their_existing_policy() {
    let mut engine = Engine::default();
    for code in [
        "x=inv(single(zeros(2)));",
        "x=single(zeros(2))\\single([1;2]);",
        "x=mldivide(single(zeros(2)),single([1;2]));",
        "x=mrdivide(single([1 2]),single(zeros(2)));",
        "x=mldivide(int32(zeros(2)),int32([1;2]));",
    ] {
        assert!(
            engine.execute(code).unwrap_err().contains("singular"),
            "{code}"
        );
    }
    assert!(
        engine
            .execute("x=inv(single(eye(3)));")
            .unwrap()
            .output
            .is_empty()
    );
}

#[test]
fn real_singular_triangular_condition_estimate_and_rhs_shapes() {
    let mut engine = Engine::default();
    let close = "Warning: Matrix is close to singular or badly scaled. Results may be inaccurate. RCOND = 1.000000e-20.";
    for code in [
        "x=[1 0;0 1e-20]\\eye(2);",
        "x=eye(2)/[1 0;0 1e-20];",
        "x=mldivide([1 0;0 1e-20],eye(2));",
        "x=inv([1 0;0 1e-20]);",
    ] {
        assert_eq!(engine.execute(code).unwrap().output, [close], "{code}");
        same_values(
            &engine.variables["x"],
            &Matrix::from_column_vec(2, 2, vec![1., 0., 0., 1e20]).unwrap(),
        );
    }
    for code in ["x=[1 2;3 4]\\[1;2;3];", "x=[1 2 3]/[1 2;3 4];"] {
        assert!(engine.execute(code).is_err(), "{code}");
    }
    assert!(
        engine
            .execute("x=inv(single(magic(4)));")
            .unwrap()
            .output
            .is_empty()
    );
    assert!(
        engine
            .execute("x=mldivide(single(magic(4)),single([1;2;3;4]));")
            .unwrap()
            .output
            .is_empty()
    );
}
