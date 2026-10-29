//! Replays the core math cases in `tests/fixtures/fn/core-cases.txt` through
//! MatRust: console text byte for byte, then
//! the class and size of `x`; an error case compares the message.
//!
//! Block format: `@@CASE <id>|<statement>`, the `evalc` console text,
//! `@@CLASS <class> <size>` (absent on error), `@@END`. An error case holds
//! `ERR <message>` instead. Case 34 ran under `format long`.
use matrust::Engine;

const REFERENCE: &str = include_str!("fixtures/fn/core-expected.txt");

/// Cases that ran under `format long`.
const LONG: &[&str] = &["34"];

/// Cases MatRust does not match yet, with the reason. Empty: every case passes.
const KNOWN: &[(&str, &str)] = &[];

struct Case {
    id: String,
    statement: String,
    expected: String,
    class: Option<String>,
    error: Option<String>,
}

fn cases(reference: &str) -> Vec<Case> {
    let reference = reference.replace("\r\n", "\n");
    let mut cases = Vec::new();
    let mut rest = reference.as_str();
    while let Some(start) = rest.find("@@CASE ") {
        rest = &rest[start + "@@CASE ".len()..];
        let (header, after) = rest.split_once('\n').expect("case header line");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (body, after) = after.split_once("@@END\n").expect("case end marker");
        let (expected, class, error) = if let Some(message) = body.strip_prefix("ERR ") {
            (String::new(), None, Some(message.trim_end().to_string()))
        } else {
            let (text, class) = body.rsplit_once("@@CLASS ").expect("class line");
            (text.to_string(), Some(class.trim_end().to_string()), None)
        };
        cases.push(Case {
            id: id.into(),
            statement: statement.into(),
            expected,
            class,
            error,
        });
        rest = after;
    }
    cases
}

/// MATLAB's `mat2str(size(x))` of a two-dimensional array.
fn class_and_size(engine: &Engine) -> String {
    match engine.array("x") {
        Some(x) => format!("{} [{} {}]", x.class(), x.rows(), x.cols()),
        None => "<no x>".into(),
    }
}

/// MatRust prefixes an error with its location; MATLAB's message is the rest.
fn message(error: &str) -> &str {
    let Some(rest) = error.strip_prefix("Line ") else {
        return error;
    };
    match rest.split_once(": ") {
        Some((location, message)) if location.contains(", column ") => message,
        _ => error,
    }
}

fn replay(case: &Case) -> std::result::Result<(), String> {
    let mut engine = Engine::default();
    if LONG.contains(&case.id.as_str()) {
        engine.execute("format long").expect("format long");
    }
    let result = engine.execute(&case.statement);
    match (&case.error, result) {
        (Some(expected), Err(actual)) if message(&actual) == expected => Ok(()),
        (Some(expected), Err(actual)) => Err(format!(
            "expected error {expected:?}\n  actual error   {actual:?}"
        )),
        (Some(message), Ok(_)) => Err(format!("expected error {message:?}, but it ran")),
        (None, Err(actual)) => Err(format!("unexpected error {actual:?}")),
        (None, Ok(result)) => {
            let actual: String = result
                .output
                .iter()
                .map(|entry| format!("{entry}\n"))
                .collect();
            if actual != case.expected {
                return Err(format!(
                    "expected {:?}\n  actual   {:?}",
                    case.expected, actual
                ));
            }
            let class = class_and_size(&engine);
            if Some(&class) != case.class.as_ref() {
                return Err(format!("expected class {:?}, actual {class:?}", case.class));
            }
            Ok(())
        }
    }
}

#[test]
fn corpus_has_every_case() {
    let all = cases(REFERENCE);
    assert_eq!(all.len(), 82);
    for (id, _) in KNOWN {
        assert!(all.iter().any(|c| c.id == *id), "known case {id} exists");
    }
    for id in LONG {
        assert!(all.iter().any(|c| c.id == *id), "long case {id} exists");
    }
}

#[test]
fn matches_reference_core_math() {
    let mut failures = Vec::new();
    let mut passed = 0;
    let mut known_passing = Vec::new();
    for case in cases(REFERENCE) {
        let known = KNOWN.iter().any(|(id, _)| *id == case.id);
        match replay(&case) {
            Ok(()) if known => known_passing.push(case.id.clone()),
            Ok(()) => passed += 1,
            Err(_) if known => {}
            Err(message) => failures.push(format!(
                "case {} `{}`\n  {message}",
                case.id, case.statement
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "{passed} passed, {} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        known_passing.is_empty(),
        "KNOWN cases now pass; remove them: {known_passing:?}"
    );
    assert_eq!(passed + KNOWN.len(), 82);
}

fn run(source: &str) -> std::result::Result<String, String> {
    let mut engine = Engine::default();
    engine
        .execute(source)
        .map(|r| r.output.iter().map(|e| format!("{e}\n")).collect())
}

/// Beyond the corpus: checks MatRust's own guarantees, not reference text.
#[test]
fn narginchk_checks_the_calling_function() {
    let source = "function r = f(a, b, c)\nnarginchk(1, 2);\nr = nargin;\nend\n";
    assert_eq!(
        run(&format!("x = f(1)\n{source}")).unwrap(),
        "\nx =\n\n     1\n\n"
    );
    let few = run(&format!("f()\n{source}")).unwrap_err();
    assert!(few.ends_with("Not enough input arguments."), "{few}");
    let many = run(&format!("f(1, 2, 3)\n{source}")).unwrap_err();
    assert!(many.ends_with("Too many input arguments."), "{many}");
}

#[test]
fn operator_forms_check_their_arity() {
    assert!(
        run("plus(1)")
            .unwrap_err()
            .ends_with("Not enough input arguments.")
    );
    assert!(
        run("plus(1, 2, 3)")
            .unwrap_err()
            .ends_with("Too many input arguments.")
    );
    assert!(
        run("uminus(1, 2)")
            .unwrap_err()
            .ends_with("Too many input arguments.")
    );
}

#[test]
fn large_arguments_stay_finite() {
    let mut engine = Engine::default();
    engine
        .execute("t = tan(1+400i); s = sqrt(1e308*(-1+1i)); a = acos(1e200+1e200i);")
        .unwrap();
    for name in ["t", "s", "a"] {
        let matrust::Array::Complex(z) = engine.array(name).unwrap() else {
            panic!("{name} is complex");
        };
        let z = z.data()[0];
        assert!(z.re.is_finite() && z.im.is_finite(), "{name} = {z}");
    }
}

#[test]
fn matrix_powers() {
    // Real A^p without negative eigenvalues is real; (A^0.5)^2 = A.
    let mut engine = Engine::default();
    engine
        .execute("r = [2 1; 1 2]^0.5; d = r*r - [2 1; 1 2]; c = [1 1i; 0 1]^-2;")
        .unwrap();
    let matrust::Array::Numeric(d) = engine.array("d").unwrap() else {
        panic!("d is real");
    };
    assert!(d.data().iter().all(|v| v.abs() < 1e-14), "{:?}", d.data());
    let matrust::Array::Complex(c) = engine.array("c").unwrap() else {
        panic!("c is complex");
    };
    assert_eq!(c.data()[2], matrust::Complex64::new(0., -2.));
    assert!(
        run("x = [1 2i]^2")
            .unwrap_err()
            .ends_with("Inputs must be a scalar and a square matrix.")
    );
}

// Additional reference observations supplied by the orchestrator for fix round 1
// (format long, 2026-10-23). These are separate from the original 82-case corpus.
fn assert_relative(actual: f64, expected: f64) {
    if expected.is_infinite() || expected == 0. {
        assert_eq!(actual, expected);
    } else {
        // Division keeps the tolerance meaningful even for subnormal results.
        assert!(
            (actual / expected - 1.).abs() <= 1e-14,
            "actual {actual:.17e}, expected {expected:.17e}"
        );
    }
}

fn assert_reference_value(
    source: &str,
    rows: usize,
    cols: usize,
    expected: &[(f64, f64)],
) -> Vec<String> {
    let mut engine = Engine::default();
    let execution = engine.execute(&format!("x = {source};")).unwrap();
    let actual = engine.array("x").unwrap();
    assert_eq!((actual.rows(), actual.cols()), (rows, cols));
    match actual {
        matrust::Array::Numeric(m) => {
            assert_eq!(m.len(), expected.len());
            for (&a, &(re, im)) in m.data().iter().zip(expected) {
                assert_relative(a, re);
                assert_eq!(im, 0.);
            }
        }
        matrust::Array::Complex(m) => {
            assert!(
                expected.iter().any(|&(_, im)| im != 0.),
                "must narrow to real"
            );
            assert_eq!(m.len(), expected.len());
            for (a, &(re, im)) in m.data().iter().zip(expected) {
                assert_relative(a.re, re);
                assert_relative(a.im, im);
            }
        }
        _ => panic!("expected double"),
    }
    execution.output
}

#[test]
fn reference_defective_matrix_power_warns_and_returns_identity() {
    let identity = [
        (1., 0.),
        (0., 0.),
        (0., 0.),
        (0., 0.),
        (1., 0.),
        (0., 0.),
        (0., 0.),
        (0., 0.),
        (1., 0.),
    ];
    for source in [
        "[1 1 0; 0 1 1; 0 0 1]^0.5",
        "mpower([1 1 0; 0 1 1; 0 0 1], 0.5)",
    ] {
        let output = assert_reference_value(source, 3, 3, &identity);
        assert_eq!(output.len(), 1, "{output:?}");
        let rcond = output[0].strip_prefix("Warning: Matrix is close to singular or badly scaled. Results may be inaccurate. RCOND = ").expect("MATLAB warning text").strip_suffix('.').unwrap().parse::<f64>().unwrap();
        // The numerical condition estimate depends on the eigenvector/solve
        // implementation; the reference reported 4.930381e-32 for this input.
        assert!(rcond > 0. && rcond < f64::EPSILON, "{rcond}");
    }
}

#[test]
fn reference_zero_matrix_fractional_power() {
    assert!(assert_reference_value("zeros(2)^0.5", 2, 2, &[(0., 0.); 4]).is_empty());
}

#[test]
fn reference_del2_three_samples() {
    for source in ["del2([1 4 9])", "del2([1 4 9]')'"] {
        assert_reference_value(source, 1, 3, &[(0.5, 0.); 3]);
    }
    assert_reference_value("del2([1 4 9], 2)", 1, 3, &[(0.125, 0.); 3]);
}

#[test]
fn reference_complex_tan_near_pole() {
    assert_reference_value(
        "tan(pi/2 + 1e-10i)",
        1,
        1,
        &[(6.123_233_995_734_469e3, 9.999_999_999_996_25e9)],
    );
}

#[test]
fn reference_complex_sqrt_near_overflow() {
    assert_reference_value(
        "sqrt(1.7e308 + 1.7e308i)",
        1,
        1,
        &[(1.432_508_823_015_457e154, 5.933_645_827_121_221e153)],
    );
}

#[test]
fn reference_complex_reciprocal_subnormal() {
    assert_reference_value("(1e308 + 1e308i)^-1", 1, 1, &[(5e-309, -5e-309)]);
}

#[test]
#[allow(clippy::approx_constant)] // Preserve the supplied reference format-long value.
fn reference_acos_retains_reference_overflow() {
    assert_reference_value(
        "acos(1.4e308 + 1.4e308i)",
        1,
        1,
        &[(0.785_398_163_397_448, f64::NEG_INFINITY)],
    );
}

#[test]
fn reference_realpow_empty_scalar_expansion() {
    for source in ["realpow([], 2)", "realpow(2, [])", "realpow([], [])"] {
        assert_reference_value(source, 0, 0, &[]);
    }
    assert_reference_value("realpow(zeros(0,3), 2)", 0, 3, &[]);
    assert_reference_value("realpow(2, zeros(3,0))", 3, 0, &[]);
}

#[test]
#[ignore = "release benchmark; run separately with CORE_MATH_BENCH_WORKERS=1 and 4"]
fn benchmark_fix_round1() {
    let workers = std::env::var("CORE_MATH_BENCH_WORKERS")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    for (label, setup, expression) in [
        (
            "sqrt 1e6 complex",
            "a = (1.7e308+1.7e308i)*ones(1,1000000);",
            "r = sqrt(a);",
        ),
        (
            "tan 1e6 complex",
            "a = (pi/2+1e-10i)*ones(1,1000000);",
            "r = tan(a);",
        ),
        (
            "reciprocal 1e6 complex",
            "a = (1e308+1e308i)*ones(1,1000000);",
            "r = a.^-1;",
        ),
        ("gradient 1e6 vector", "a = 1:1000000;", "r = gradient(a);"),
        ("del2 1000x1000", "a = ones(1000);", "r = del2(a);"),
        ("unwrap 1000x1000", "a = ones(1000);", "r = unwrap(a);"),
        (
            "mpower 100x100",
            "a = diag(1:100) + ones(100)/100;",
            "r = a^0.5;",
        ),
    ] {
        let mut engine = Engine::default();
        engine.execute(setup).unwrap();
        engine.execute(expression).unwrap();
        let mut times = Vec::new();
        for _ in 0..5 {
            let start = std::time::Instant::now();
            engine.execute(expression).unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "workers={workers}, {label}, median_ms={:.3}, repetitions=5",
            times[2]
        );
    }
}
