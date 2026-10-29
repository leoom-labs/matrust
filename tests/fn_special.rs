//! Replays the reference's console text (`evalc` of each statement) for the
//! special functions (Bessel, Airy, incomplete gamma/beta and inverses,
//! expint, psi, legendre, elliptic integrals and functions, erfcx), plus the
//! class and size of `x`.
//!
//! `tests/fixtures/fn/special-expected.txt` holds every case of
//! `tests/fixtures/fn/special-cases.txt` with expected outputs. Block format: `@@CASE <id>|<statement>`,
//! the console text, `@@CLASS <class> <size>` (absent on error), `@@END`; an
//! error case has `ERR <message>`. Cases marked `LONG` in the case list ran
//! under `format long`.
//!
//! Numeric rule for this group (task brief): when the console text differs
//! from the corpus only in numbers, both are parsed and a relative difference
//! up to 1e-13 (absolute 1e-300 near zero) is accepted; the text between the
//! numbers (layout, signs, `i`), errors, class and size stay exact. Every case
//! that needed the rule is listed in `HARNESS` with its maximum relative error,
//! and the test prints each one.
use matrust::Engine;

const REFERENCE: &str = include_str!("fixtures/fn/special-expected.txt");
const CASES: usize = 130;

/// The case ids `tests/fixtures/fn/special-cases.txt` marks `LONG` (all but these).
const SHORT: &[&str] = &[
    "15", "53", "66", "72", "80", "81", "86", "101", "108", "109", "115", "116", "122", "123",
    "130",
];

/// Cases that cannot pass for a reason outside this task.
const KNOWN: &[(&str, &str)] = &[
    (
        "64",
        "the reference's rounding prints 4.000000000000001 for the exact 4; MatRust computes 4 exactly, \
         which displays as the integer `4` (a layout difference the numeric rule cannot accept)",
    ),
    (
        "107",
        "legendre of a 2x2 X returns a 2x2x2 array; MatRust has no N-D arrays (it errors explicitly)",
    ),
];

const RELATIVE: f64 = 1e-13;
const ABSOLUTE: f64 = 1e-300;

/// Cases compared under the numeric rule, with the largest relative error seen
/// on Linux. Every other case must match byte for byte; a listed case may
/// also match exactly (the last bit of libm differs between platforms).
const HARNESS: &[(&str, &str)] = &[
    ("1", "relative 1e-13; max 1.306e-15 on Linux"),
    (
        "2",
        "relative 1e-13; max 2.298e-14 in clean-room kernel probe",
    ),
    ("3", "relative 1e-13; max 3.842e-15 on Linux"),
    ("7", "relative 1e-13; max 7.755e-16 on Linux"),
    // Clean-room Temme/Steed rewrite: independent recurrence and stopping
    // choices change the final printed digits, with layout kept exact.
    (
        "12",
        "relative 1e-13; max 1.733e-15 in clean-room kernel probe",
    ),
    (
        "17",
        "relative 1e-13; max 9.335e-15 in clean-room kernel probe",
    ),
    ("19", "relative 1e-13; max 7.315e-16 on Linux"),
    ("25", "relative 1e-13; max 1.460e-16 on Linux"),
    (
        "30",
        "relative 1e-13; max 2.105e-15 in clean-room kernel probe",
    ),
    ("38", "relative 1e-13; max 1.113e-14 on Linux"),
    (
        "39",
        "relative 1e-13; max 1.306e-15 in clean-room kernel probe",
    ),
    ("40", "relative 1e-13; max 2.271e-15 on Linux"),
    (
        "41",
        "relative 1e-13; max 1.306e-15 in clean-room kernel probe",
    ),
    ("48", "relative 1e-13; max 7.356e-16 on Linux"),
    ("49", "relative 1e-13; max 1.072e-15 on Linux"),
    (
        "51",
        "relative 1e-13; max 9.949e-14 on Linux (reference is 1.07e-13 off the true Ai(-10))",
    ),
    (
        "57",
        "relative 1e-13; max 1.477e-15 on Linux (5 exp(-2) is correctly rounded, the reference one unit low)",
    ),
    ("59", "relative 1e-13; max 1.947e-15 on Linux"),
    ("67", "relative 1e-13; max 5.292e-16 on Linux"),
    ("68", "relative 1e-13; max 1.669e-16 on Linux"),
    ("69", "relative 1e-13; max 5.292e-16 on Linux"),
    ("70", "relative 1e-13; max 9.579e-16 on Linux"),
    ("71", "relative 1e-13; max 1.426e-16 on Linux"),
    ("77", "relative 1e-13; max 9.592e-14 on Linux"),
    ("82", "relative 1e-13; max 2.590e-15 on Linux"),
    ("84", "relative 1e-13; max 2.590e-15 on Linux"),
    ("85", "relative 1e-13; max 8.398e-15 on Linux"),
    ("87", "relative 1e-13; max 4.555e-15 on Linux"),
    ("92", "relative 1e-13; max 3.884e-16 on Linux"),
    ("94", "relative 1e-13; max 1.731e-15 on Linux"),
    ("95", "relative 1e-13; max 2.363e-15 on Linux"),
    ("97", "relative 1e-13; max 3.694e-16 on Linux"),
    ("104", "relative 1e-13; max 1.184e-16 on Linux"),
];

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

/// Splits console text into the text between numbers and the numbers.
fn tokens(text: &str) -> (Vec<String>, Vec<f64>) {
    let bytes = text.as_bytes();
    let mut layout = vec![String::new()];
    let mut numbers = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        let mut digits = j > start;
        if j < bytes.len() && bytes[j] == b'.' {
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
                digits = true;
            }
        }
        if digits {
            if j < bytes.len() && (bytes[j] == b'e' || bytes[j] == b'E') {
                let mut k = j + 1;
                if k < bytes.len() && (bytes[k] == b'+' || bytes[k] == b'-') {
                    k += 1;
                }
                if k < bytes.len() && bytes[k].is_ascii_digit() {
                    while k < bytes.len() && bytes[k].is_ascii_digit() {
                        k += 1;
                    }
                    j = k;
                }
            }
            numbers.push(text[start..j].parse().expect("number"));
            // The number's width is part of the layout.
            layout.push(format!("#{}", j - start));
            layout.push(String::new());
            i = j;
        } else {
            layout.last_mut().unwrap().push(bytes[i] as char);
            i += 1;
        }
    }
    (layout, numbers)
}

/// `None` when the texts differ beyond the numeric rule; else the largest
/// relative error (0 for identical text).
fn numeric_match(expected: &str, actual: &str) -> Option<f64> {
    if expected == actual {
        return Some(0.);
    }
    let (layout_e, numbers_e) = tokens(expected);
    let (layout_a, numbers_a) = tokens(actual);
    if layout_e != layout_a || numbers_e.len() != numbers_a.len() {
        return None;
    }
    let mut worst: f64 = 0.;
    for (e, a) in numbers_e.iter().zip(&numbers_a) {
        let difference = (e - a).abs();
        if difference <= ABSOLUTE {
            continue;
        }
        let relative = difference / e.abs().max(a.abs());
        if relative > RELATIVE {
            return None;
        }
        worst = worst.max(relative);
    }
    Some(worst)
}

/// `class(x)` and `size(x)` in the corpus form `double [1 3]`.
fn class_and_size(engine: &mut Engine) -> Result<String, String> {
    let class = engine.execute("disp(class(x))")?.output.concat();
    let size = engine.execute("disp(size(x))")?.output.concat();
    let size: Vec<_> = size.split_whitespace().collect();
    Ok(format!("{} [{}]", class.trim(), size.join(" ")))
}

/// `Ok(Some(error))` for a case that matched only under the numeric rule.
fn run(case: &Case) -> Result<Option<f64>, String> {
    let mut engine = Engine::default();
    if !SHORT.contains(&case.id.as_str()) {
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
            Err(error) if error == message => Ok(None),
            Err(error) => Err(format!("expected error {message:?}, got {error:?}")),
            Ok(_) => Err(format!("expected error {message:?}, got a value")),
        };
    }
    let actual: String = result?
        .output
        .iter()
        .map(|entry| format!("{entry}\n"))
        .collect();
    let Some(error) = numeric_match(&case.expected, &actual) else {
        return Err(format!(
            "expected {:?}\n  actual   {:?}",
            case.expected, actual
        ));
    };
    let expected = case.class.as_deref().unwrap_or_default();
    let class = class_and_size(&mut engine)?;
    if class != expected {
        return Err(format!("expected class {expected:?}, got {class:?}"));
    }
    Ok((case.expected != actual).then_some(error))
}

#[test]
fn corpus_has_every_case() {
    let all = cases();
    assert_eq!(all.len(), CASES);
    for id in SHORT {
        let case = all.iter().find(|c| c.id == *id).expect("short case exists");
        assert!(!case.statement.is_empty());
    }
    for (id, _) in KNOWN.iter().chain(HARNESS) {
        assert!(all.iter().any(|c| c.id == *id), "listed case {id} exists");
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
fn numeric_rule_keeps_layout_exact() {
    assert_eq!(numeric_match("   1.5\n", "   1.5\n"), Some(0.));
    assert!(numeric_match("   0.500000000000048\n", "   0.500000000000000\n").is_some());
    assert!(numeric_match("   0.500000000000048\n", "   0.500000000001000\n").is_none());
    assert!(numeric_match("  -1.5\n", "   1.5\n").is_none());
    assert!(numeric_match("     0\n", "   0.000000000000000\n").is_none());
    assert!(numeric_match("1 - 0.0i\n", "1 + 0.0i\n").is_none());
}

#[test]
fn matches_reference() {
    let mut failures = Vec::new();
    let mut passed = 0;
    let mut known_passing = Vec::new();
    let mut relaxed = Vec::new();
    for case in cases() {
        let outcome = run(&case);
        if KNOWN.iter().any(|(id, _)| *id == case.id) {
            if outcome.is_ok() {
                known_passing.push(case.id.clone());
            }
            continue;
        }
        match outcome {
            Ok(None) => passed += 1,
            Ok(Some(error)) => {
                passed += 1;
                println!(
                    "case {} `{}`: numeric rule, max relative error {error:.3e}",
                    case.id, case.statement
                );
                relaxed.push(case.id.clone());
            }
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
    let unlisted: Vec<_> = relaxed
        .iter()
        .filter(|id| !HARNESS.iter().any(|(listed, _)| listed == id))
        .collect();
    assert!(
        unlisted.is_empty(),
        "cases matched only under the numeric rule but missing from HARNESS: {unlisted:?}"
    );
    assert_eq!(passed, CASES - KNOWN.len());
}

/// Values away from the corpus against mpmath at 25 digits (not reference-checked):
/// `(statement, real, imaginary, relative tolerance)`.
#[test]
#[allow(clippy::excessive_precision)]
fn agrees_with_high_precision_references() {
    let cases: &[(&str, f64, f64, f64)] = &[
        ("besselj(1/3, 7.5)", 0.28967659629236255, 0., 1e-13),
        ("bessely(2.7, 0.3)", -83.569626864374428, 0., 1e-13),
        ("besselj(0, 1e5)", -0.0017192011162359722, 0., 1e-11),
        ("besselj(3, 30)", 0.12921122875972498, 0., 1e-13),
        ("besseli(2.5, 40, 1)", 0.058465711408685896, 0., 1e-13),
        ("besselk(0.25, 1e-3)", 11.756476271934459, 0., 1e-13),
        ("besselk(3, 30, 1)", 0.2640981659432364, 0., 1e-13),
        (
            "besselj(0, 10+1i)",
            -0.3817032673394245,
            -0.04642191069097068,
            1e-9,
        ),
        (
            "besselk(0, 5+5i)",
            0.001945163072458818,
            0.002460604699954409,
            1e-9,
        ),
        ("gammainc(9.9e3, 1e4)", 0.15865119219356466, 0., 1e-12),
        ("betainc(0.4, 30, 40)", 0.31814380259056299, 0., 1e-12),
        ("gammainc(0.01, 0.3)", 0.27924099635901486, 0., 1e-13),
        ("ellipke(0.7)", 2.0753631352924691, 0., 1e-14),
        ("psi(1e-3)", -1000.5755719318103, 0., 1e-14),
        ("psi(3, 0.7)", 25.879149678427738, 0., 1e-13),
        (
            "expint(-30)",
            -368973209407.2742,
            -std::f64::consts::PI,
            1e-13,
        ),
        ("airy(0, 3.5)", 0.002584098786989635, 0., 1e-13),
        ("airy(2, -4.2)", 0.38346736127094463, 0., 1e-13),
        (
            "airy(0, 2+3i)",
            0.008104457809530535,
            0.13117838260456603,
            1e-11,
        ),
    ];
    let mut failures = Vec::new();
    for &(source, re, im, tolerance) in cases {
        let mut engine = Engine::default();
        engine
            .execute(&format!("v = {source}; r = real(v); q = imag(v);"))
            .unwrap_or_else(|e| panic!("{source}: {e}"));
        let (r, q) = (
            engine.variables["r"].data()[0],
            engine.variables["q"].data()[0],
        );
        let scale = re.abs().max(im.abs());
        let error = if r.is_finite() && q.is_finite() {
            (r - re).abs().max((q - im).abs()) / scale
        } else {
            f64::INFINITY
        };
        println!("{source}: relative error {error:.2e}");
        if error > tolerance {
            failures.push(format!("{source}: {r} + {q}i, expected {re} + {im}i"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn outputs_shapes_and_errors() {
    let mut e = Engine::default();
    e.execute("[k, ee] = ellipke([0.2 0.4]); t = besselj([0 1], [1; 2; 3]);")
        .unwrap();
    assert_eq!(e.variables["ee"].cols(), 2);
    let t = &e.variables["t"];
    assert_eq!((t.rows(), t.cols()), (3, 2));
    assert!(e.execute("[a, b] = psi(1)").is_err());
    assert!(e.execute("[a, b, c, d] = ellipj(1, 0.5)").is_err());
    assert!(e.execute("y = besselj([1 2 3], [1 2])").is_err());
    assert!(e.execute("y = gammainc(1, 2, 'middle')").is_err());
    assert!(e.execute("y = legendre(2, 0.5, 'other')").is_err());
    assert!(e.execute("y = besselj(1i, 2)").is_err());
}

/// Values the reference returned for edge cases (orchestrator run, 2026-10-24), as
/// `(statement, value, relative tolerance)`; they must also be bounded in time.
#[test]
fn matches_reference_edge_values() {
    let cases: &[(&str, f64, f64)] = &[
        ("besseli(0, 710)", f64::INFINITY, 0.),
        ("bessely(-1, 0)", f64::NEG_INFINITY, 0.),
        ("gammainc(0, 2, 'scaledlower')", 1., 0.),
        ("gammainc(1e-14, 20)", 4.110_317_623_311_887e-299, 1e-12),
        ("gammaincinv(1e-300, 2)", 1.4142135623730947e-150, 1e-13),
        ("besselj(1e12, 1)", 0., 0.),
        ("besselj(Inf, 1)", 0., 0.),
        ("besselj(1e5, 1)", 0., 0.),
        ("psi(171, 1000)", 0., 0.),
        // The reference itself is 1.07e-13 from the true Ai(-10) = 0.04024123848644319;
        // its printed 15 digits (corpus case 51) are within the 1e-13 rule.
        ("airy(0, -10)", 0.040241238486438895, 1.1e-13),
    ];
    let start = std::time::Instant::now();
    for &(source, want, tolerance) in cases {
        let mut engine = Engine::default();
        engine
            .execute(&format!("v = {source};"))
            .unwrap_or_else(|e| panic!("{source}: {e}"));
        let got = engine.variables["v"].data()[0];
        let ok = if want.is_infinite() || want == 0. {
            got == want
        } else {
            (got - want).abs() <= tolerance * want.abs()
        };
        println!("{source} = {got:e} (reference {want:e})");
        assert!(ok, "{source}: got {got:e}, reference {want:e}");
    }
    assert!(
        start.elapsed().as_secs() < 5,
        "edge cases must return at once"
    );
}

#[test]
fn ellipj_memory_stays_bounded() {
    let mut engine = Engine::default();
    engine
        .execute("[s, c, d] = ellipj(1, ones(1, 1000000)); x = s(1) - tanh(1);")
        .unwrap();
    assert!(engine.variables["x"].data()[0].abs() < 1e-15);
}
