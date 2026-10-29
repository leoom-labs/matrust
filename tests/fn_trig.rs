//! Replays the reference's console text (`evalc` of each statement) for the
//! hyperbolic, reciprocal and degree trigonometry, angle-unit and coordinate
//! functions, and compares it byte for byte, plus the class and size of `x`.
//!
//! `tests/fixtures/fn/trig-expected.txt` holds every case of
//! `tests/fixtures/fn/trig-cases.txt` with expected outputs. Block format: `@@CASE <id>|<statement>`,
//! the console text, `@@CLASS <class> <size>` (absent on error), `@@END`; an
//! error case has `ERR <message>`. Cases marked `LONG` in the case list ran
//! under `format long`.
use matrust::Engine;

const REFERENCE: &str = include_str!("fixtures/fn/trig-expected.txt");

/// The cases `tests/fixtures/fn/trig-cases.txt` marks `LONG`.
const LONG: &[(&str, &str)] = &[
    ("23", "x = sind(30)"),
    ("24", "x = cosd(60)"),
    ("47", "x = deg2rad(180)"),
];

/// Cases that cannot pass for a reason outside this task.
const KNOWN: &[(&str, &str)] = &[];

struct Case {
    id: String,
    statement: String,
    expected: String,
    class: Option<String>,
}

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    let mut rest = REFERENCE;
    while let Some(start) = rest.find("@@CASE ") {
        rest = &rest[start + "@@CASE ".len()..];
        let (header, after) = rest.split_once('\n').expect("case header line");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (body, after) = after.split_once("@@END\n").expect("case end marker");
        let (expected, class) = match body.split_once("@@CLASS ") {
            Some((text, class)) => (text, Some(class.trim_end().to_string())),
            None => (body, None),
        };
        cases.push(Case {
            id: id.into(),
            statement: statement.into(),
            expected: expected.into(),
            class,
        });
        rest = after;
    }
    cases
}

/// `class(x)` and `size(x)` in the corpus form `double [1 3]`.
fn class_and_size(engine: &mut Engine) -> Result<String, String> {
    let class = engine.execute("disp(class(x))")?.output.concat();
    let size = engine.execute("disp(size(x))")?.output.concat();
    let size: Vec<_> = size.split_whitespace().collect();
    Ok(format!("{} [{}]", class.trim(), size.join(" ")))
}

fn run(case: &Case) -> Result<(), String> {
    let mut engine = Engine::default();
    if LONG.iter().any(|(id, _)| *id == case.id) {
        engine.execute("format long")?;
    }
    let result = engine.execute(&case.statement);
    if let Some(message) = case.expected.strip_prefix("ERR ") {
        let message = message.trim_end();
        // MatRust prefixes the statement position, which `evalc` text lacks.
        let result = result.map_err(|error| match error.split_once(": ") {
            Some((place, text)) if place.starts_with("Line ") => text.to_string(),
            _ => error,
        });
        return match result {
            Err(error) if error == message => Ok(()),
            Err(error) => Err(format!("expected error {message:?}, got {error:?}")),
            Ok(_) => Err(format!("expected error {message:?}, got a value")),
        };
    }
    let actual: String = result?
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
    let expected = case.class.as_deref().unwrap_or_default();
    let class = class_and_size(&mut engine)?;
    if class != expected {
        return Err(format!("expected class {expected:?}, got {class:?}"));
    }
    Ok(())
}

#[test]
fn corpus_has_every_case() {
    let all = cases();
    assert_eq!(all.len(), 61);
    for (id, statement) in LONG {
        assert!(
            all.iter().any(|c| c.id == *id && c.statement == *statement),
            "LONG case {id} is `{statement}`"
        );
    }
    for (id, _) in KNOWN {
        assert!(all.iter().any(|c| c.id == *id), "known case {id} exists");
    }
    for case in &all {
        assert_eq!(
            case.class.is_none(),
            case.expected.starts_with("ERR "),
            "case {} has a class exactly when it is not an error",
            case.id
        );
    }
}

#[test]
fn matches_reference_byte_for_byte() {
    let mut failures = Vec::new();
    let mut passed = 0;
    let mut known_passing = Vec::new();
    for case in cases() {
        let outcome = run(&case);
        if KNOWN.iter().any(|(id, _)| *id == case.id) {
            if outcome.is_ok() {
                known_passing.push(case.id.clone());
            }
            continue;
        }
        match outcome {
            Ok(()) => passed += 1,
            Err(error) => {
                failures.push(format!("case {} `{}`\n  {error}", case.id, case.statement))
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} passed, {} failed:\n{}",
        passed,
        failures.len(),
        failures.join("\n")
    );
    assert!(
        known_passing.is_empty(),
        "KNOWN cases now pass; remove them from KNOWN: {known_passing:?}"
    );
    assert_eq!(passed, 61 - KNOWN.len());
}

/// Not reference-checked: identities that follow from the reference's definitions.
#[test]
fn definitions_hold_beyond_the_corpus() {
    let mut e = Engine::default();
    let value = |e: &mut Engine, source: &str| -> Vec<f64> {
        e.execute(&format!("v = {source};")).unwrap();
        e.variables["v"].data().to_vec()
    };
    assert_eq!(value(&mut e, "sind([-180 -90 540 720])"), [0., -1., 0., 0.]);
    assert_eq!(value(&mut e, "cosd([-180 450 720])"), [-1., 0., 1.]);
    assert_eq!(
        value(&mut e, "tand([-90 225 -135])"),
        [f64::NEG_INFINITY, 1., 1.]
    );
    assert!(value(&mut e, "sind([Inf NaN])").iter().all(|v| v.is_nan()));
    assert_eq!(value(&mut e, "acot(0)"), [std::f64::consts::FRAC_PI_2]);
    assert_eq!(
        value(&mut e, "cart2pol([3 0], 4)"),
        [4_f64.atan2(3.), std::f64::consts::FRAC_PI_2]
    );
    // complex in, real out narrows as arithmetic does
    assert_eq!(value(&mut e, "sinh(complex(0, 0))"), [0.]);
    assert!(e.execute("y = sind(1i)").is_err());
    assert!(e.execute("[a, b, c] = cart2pol(1, 2)").is_err());
    assert!(e.execute("y = cart2pol([1 2], [1 2 3])").is_err());
    assert!(e.execute("y = sinh(1, 2)").is_err());
}

fn assert_relative(actual: f64, expected: f64, context: &str) {
    if expected == 0. {
        assert_eq!(actual, 0., "{context}");
    } else {
        assert!(
            (actual - expected).abs() <= 1e-14 * expected.abs(),
            "{context}: expected {expected:e}, got {actual:e}"
        );
    }
}

fn assert_values(source: &str, expected: &[(f64, f64)]) {
    let mut engine = Engine::default();
    engine.execute(&format!("v = {source};")).unwrap();
    if expected.iter().all(|&(_, im)| im == 0.) {
        assert!(!engine.complex_variables.contains_key("v"), "{source}");
        let actual = &engine.variables["v"];
        assert_eq!((actual.rows(), actual.cols()), (1, expected.len()));
        for (&actual, &(re, _)) in actual.data().iter().zip(expected) {
            assert_relative(actual, re, source);
        }
    } else {
        let actual = &engine.complex_variables["v"];
        assert_eq!((actual.rows(), actual.cols()), (1, expected.len()));
        for (actual, &(re, im)) in actual.data().iter().zip(expected) {
            assert_relative(actual.re, re, source);
            assert_relative(actual.im, im, source);
        }
    }
}

// Additional reference `format long` results supplied by the orchestrator in fix
// round 1 (2026-10-23). These supplement, and do not modify, the 61-case corpus.
#[test]
#[allow(clippy::excessive_precision, clippy::approx_constant)]
fn reference_mixed_real_domains() {
    assert_values("asind([0 2])", &[(0., 0.), (90., -75.456129290216893)]);
    assert_values(
        "atanh([0.5 2])",
        &[
            (0.549306144334055, 0.),
            (0.549306144334055, 1.570796326794897),
        ],
    );
    assert_values(
        "acosh([2 0.5])",
        &[(1.316957896924817, 0.), (0., 1.047197551196598)],
    );
    assert_values(
        "asec([2 0.5])",
        &[(1.047197551196598, 0.), (0., 1.316957896924817)],
    );
    assert_values(
        "acoth([2 0.5])",
        &[
            (0.549306144334055, 0.),
            (0.549306144334055, 1.570796326794897),
        ],
    );
    for source in ["acosd([0.5 2])", "asecd([2 0.5])"] {
        assert_values(
            source,
            &[(60.000000000000007, 0.), (0., 75.456129290216893)],
        );
    }
}

#[test]
fn reference_large_complex_tanh_and_coth() {
    for source in ["tanh(400+1i)", "coth(400+1i)"] {
        assert_values(source, &[(1., 0.)]);
    }
}

#[test]
#[allow(clippy::excessive_precision)]
fn reference_negative_complex_asinh() {
    assert_values("asinh(complex(-1e10,0))", &[(-23.718998110500401, 0.)]);
}

#[test]
fn reference_small_complex_reciprocals() {
    for source in ["sech(400+1i)", "csch(400+1i)"] {
        assert_values(source, &[(2.069541098466265e-174, -3.223119293242153e-174)]);
    }
}

/// Mathematical checks, not additional MATLAB reference evidence.
#[test]
fn stable_complex_symmetry_and_reciprocal_input() {
    for source in ["tanh(-400+1i)", "coth(-400+1i)"] {
        assert_values(source, &[(-1., 0.)]);
    }
    // For tiny real arguments atan(x) rounds to x. These exercise the input
    // reciprocal when the original denominator's square overflows or underflows.
    assert_values("acot(complex(1e200,0))", &[(1e-200, 0.)]);
    assert_values(
        "acot(complex(1e-200,0))",
        &[(std::f64::consts::FRAC_PI_2, 0.)],
    );
    let mut engine = Engine::default();
    engine
        .execute("a = asinh(-1e10+1i); b = -asinh(1e10-1i);")
        .unwrap();
    let a = engine.complex_variables["a"].data()[0];
    let b = engine.complex_variables["b"].data()[0];
    assert_relative(a.re, b.re, "asinh odd symmetry (real)");
    assert_relative(a.im, b.im, "asinh odd symmetry (imaginary)");
}
