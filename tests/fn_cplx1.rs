//! Reference cplx1 corpus (complex division, linear solves, inv, det, lu,
//! qr, chol, svd, norm, rank, cond, pinv, null, orth, expm, trace, complex
//! powers), stored as expected values. The LONG flags come from the case list
//! `fixtures/fn/cplx1-cases.txt`; the
//! captured fixture headers omit them. No reference is contacted by this test.
//! See docs/tasks/2026-10-25-fn-cplx1.md.
use matrust::Engine;

const REFERENCE: &str = include_str!("fixtures/fn/cplx1-expected.txt");
const CASE_LIST: &str = include_str!("fixtures/fn/cplx1-cases.txt");
const TOTAL: usize = 178;

/// Cases that cannot pass, with the reason. Each one still runs; a KNOWN case
/// that starts to pass is reported as stale.
const KNOWN: &[(usize, &str)] = &[
    (
        55,
        "display only: the reference prints negative zeros, x(1,2) and x(2,1) real part -0.000000000000000 and x(2,2) imaginary part - 0.000000000000000i; we print +0 there",
    ),
    (
        89,
        "triu of complex input is not supported (in no complex group); qr(A) itself passes (88)",
    ),
    (
        115,
        "nalgebra's SVD returns exactly [2; 0], so the column prints as integers (`     2` / `     0`); the reference's zgesvd gives about 2 + 4e-16 and 1e-16 and prints 2.000000000000000 / 0.000000000000000",
    ),
    (
        117,
        "raw U: singular vectors are unique only up to a unit phase per column; nalgebra's U is -1 times the reference's in both columns (0.375395863444523 + 0.200722667322533i for -0.375395863444522 - 0.200722667322533i at (1,1)); abs(U), size and A - U*S*V' are compared in 118-125",
    ),
    (
        148,
        "cond of a singular matrix: nalgebra's SVD returns an exactly zero second singular value, so cond is Inf; the reference's zgesvd gives about 3.4e-17 and prints 5.9618e+16",
    ),
    (
        157,
        "raw null basis: unique only up to a unit phase; ours is -1 times the reference's (-0.707106781186547 + 0.000000000000000i, 0.000000000000000 - 0.707106781186548i); abs, size and A*N are compared in 158-162",
    ),
    (
        163,
        "null(A,'r') (rational basis) is not implemented for real input either (Too many input arguments.)",
    ),
    (
        165,
        "raw orth basis: unique only up to a unit phase; ours is -1 times the reference's (0.707106781186547 + 0.000000000000000i, 0.000000000000000 + 0.707106781186548i); abs, size and Q'*Q are compared in 166-168",
    ),
];

/// Comparison rules other than byte-exact console text.
const ERROR_MESSAGE: &str = "Compare Diagnostic.message exactly, excluding the engine's line/column wrapper; the oracle captured MException.message.";
const LAST_DIGIT: &str = "Each numeric field (sign and fixed-width padding included), with the displayed scale applied, must agree with MATLAB within 1e-13 relative or 1e-15 absolute near zero. Field end positions, nonnumeric text, class and size are exact. Includes MKL and input qr/chol rounding.";
const WARNING_LINE: &str = "Compare the [Warning: ...] text exactly without its backtrace (it names the oracle script and folder), then the rest of the console text exactly.";
const HARNESS: &[(usize, &str)] = &[
    // Last digit: 0.079545454545455 for the reference's 0.079545454545454 (x(2) imaginary
    // part; exactly 7/88 = 0.0795454545454545..., so the reference's value is one ulp low).
    (40, LAST_DIGIT),
    (45, WARNING_LINE),
    (46, WARNING_LINE),
    (47, WARNING_LINE),
    (48, WARNING_LINE),
    (49, WARNING_LINE),
    (51, ERROR_MESSAGE),
    (52, ERROR_MESSAGE),
    (53, ERROR_MESSAGE),
    // Windows only, last digit: -0.300075221748571 for the reference's -0.300075221748570 (x(1,2) real part).
    (54, LAST_DIGIT),
    (63, WARNING_LINE),
    (64, ERROR_MESSAGE),
    (71, ERROR_MESSAGE),
    // Last digit: U(3,3) real part 2.324324324324325 for the reference's 2.324324324324324 (86/37; MKL's rounding).
    (77, LAST_DIGIT),
    (105, ERROR_MESSAGE),
    (108, ERROR_MESSAGE),
    (109, ERROR_MESSAGE),
    (110, ERROR_MESSAGE),
    // nalgebra's SVD (as the real svd path) against the reference's zgesvd: singular
    // values 5.659701809018660 / 0.983755779139788 for 5.659701809018658 /
    // 0.983755779139789 (113), 5.628888623996582 for 5.628888623996581 (116),
    // 5.659701809018661 / 0.983755779139788 (122); abs(U) 0.425689609302754 /
    // 0.425689609302753 with the last digits swapped (118); abs(orth)
    // 0.707106781186548 for 0.707106781186547 at (2,1) (166).
    (113, LAST_DIGIT),
    (116, LAST_DIGIT),
    (118, LAST_DIGIT),
    (122, LAST_DIGIT),
    (139, ERROR_MESSAGE),
    // nalgebra LU inverse: 9.391485505499118 for 9.391485505499116.
    (150, LAST_DIGIT),
    // pinv through nalgebra's SVD: (1,2) imaginary part + 0.000000000000000i
    // for - 0.000000000000000i (154); last digits 0.692307692307693 +
    // 0.461538461538462i for 0.692307692307692 + 0.461538461538461i and
    // -0.461538461538462 - 0.307692307692308i for -0.461538461538461 -
    // 0.307692307692307i (155).
    (154, LAST_DIGIT),
    (155, LAST_DIGIT),
    (166, LAST_DIGIT),
    // nalgebra products and LU: (1,2) 0.841470984807896i for 0.841470984807897i.
    (169, LAST_DIGIT),
    (174, ERROR_MESSAGE),
    (177, ERROR_MESSAGE),
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

fn console_matches(id: usize, actual: &str, expected: &str) -> bool {
    match rule(id) {
        Some(r) if r == LAST_DIGIT => last_digits_match(actual, expected),
        Some(r) if r == WARNING_LINE => actual == warning_only(expected),
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
        if !KNOWN.iter().any(|&(n, _)| n == case.id) {
            assert_eq!(
                case.output.contains("[\u{8}Warning: "),
                rule(case.id) == Some(WARNING_LINE),
                "case {}",
                case.id
            );
        }
        if rule(case.id) == Some(ERROR_MESSAGE) {
            assert!(case.class_size.is_none(), "case {}", case.id);
        } else if case.class_size.is_none() {
            assert!(
                KNOWN.iter().any(|&(n, _)| n == case.id),
                "error case {} needs an expected-error rule or a KNOWN entry",
                case.id
            );
        }
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
fn cplx1_matches_reference() {
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

#[test]
fn complex_division_rejects_unsupported_operands_without_panicking() {
    for statement in [
        "x = mrdivide({1}, 1i);",
        "x = mldivide(1i, int8(2));",
        "x = single(2) / 1i;",
        "x = {1} / 1i;",
    ] {
        let mut engine = Engine::default();
        assert!(engine.execute(statement).is_err(), "{statement}");
    }
}

/// 1x1 inv follows the scalar operator: nalgebra's complex division squares
/// the components, which gave 0 and Inf here (reference: 5.0000e-201 - 5.0000e-201i
/// and 5.0000e+199 - 5.0000e+199i).
#[test]
fn inv_of_a_scalar_survives_extreme_magnitudes() {
    for (statement, expected) in [
        ("z = 1e200+1e200i; x = inv(z); w = 1/z;", 5e-201),
        ("z = 1e-200+1e-200i; x = inv(z); w = 1/z;", 5e199),
    ] {
        let mut engine = Engine::default();
        engine.execute(statement).unwrap();
        let parts = |name: &str| match engine.array(name) {
            Some(matrust::Array::Complex(z)) => {
                assert_eq!((z.rows(), z.cols()), (1, 1), "{statement}");
                (z.data()[0].re, z.data()[0].im)
            }
            other => panic!("{statement}: {name} is {other:?}"),
        };
        let (re, im) = parts("x");
        assert_eq!(parts("w"), (re, im), "{statement}: inv(z) differs from 1/z");
        assert!(
            (re / expected - 1.).abs() < 1e-14 && (im / -expected - 1.).abs() < 1e-14,
            "{statement}: got {re:e} {im:e}"
        );
    }
}

#[test]
fn complex_qr_chol_and_least_squares_forms_beyond_the_corpus() {
    let value = |statements: &str| {
        let mut engine = Engine::default();
        let result = engine.execute(statements).unwrap();
        let x = engine.array("x").unwrap();
        (format!("{:?}", x), result.output.join("\n"))
    };
    for statement in [
        "A = [1 1i 2; 2 1 0; 1i 3 1; 0 1 1i]; [Q, R, p] = qr(A, 0); x = max(max(abs(Q*R - A(:, p)))) < 1e-12 && isequal(size(Q), [4 3]);",
        "A = [1 1i 2; 2 1 0; 1i 3 1; 0 1 1i]; [Q, R, E] = qr(A); x = max(max(abs(Q*R - A*E))) < 1e-12 && isequal(size(R), [4 3]);",
        "A = [1 1i 2; 2 1 0; 1i 3 1]; [Q, R, p] = qr(A, 'vector'); x = isequal(p, [2 1 3]) && max(max(abs(Q*R - A(:, p)))) < 1e-12;",
        "A = [5 1+1i 0; 1-1i 4 1i; 0 -1i 3]; L = chol(A, 'lower'); x = max(max(abs(L*L' - A))) < 1e-12;",
        "[L, p] = chol([1 2i; -2i 1], 'lower'); x = isequal([p L], [2 1]);",
        "x = isequal(mrdivide([1 2 3], [1 1i 0; 0 1 1i]), [1 2 3]/[1 1i 0; 0 1 1i]);",
        "A = [1i 1; 2 1i; 3 1; 4 2]; b = [1 2; 1i 0; 2 1; 3 1i]; y = A\\b; x = max(max(abs(A'*(A*y - b)))) < 1e-12;",
    ] {
        let (x, _) = value(statement);
        assert!(
            x.contains("Logical") && x.contains("data: [1]"),
            "{statement}: {x}"
        );
    }
    let (_, output) = value("x = mldivide([1 1i; 2 2i; 3 3i], [1 1; 2 2; 3 3]);");
    assert!(
        output.contains("Warning: Rank deficient, rank = 1, tol ="),
        "{output}"
    );
    let mut engine = Engine::default();
    for (statement, message) in [
        ("x = chol([1i 0; 0 1], 'lower');", "real diagonal"),
        ("x = qr([1 1i], 2);", "economy-size"),
        ("x = chol([1 1i], 'upper');", "must be square"),
        ("x = [1 1i; 1 2; 1i 3] \\ [1; 2];", "dimensions must agree"),
    ] {
        let error = engine.execute(statement).unwrap_err();
        assert!(error.contains(message), "{statement}: {error}");
    }
}

#[test]
fn complex_svd_family_beyond_the_corpus() {
    let truth = |statement: &str| {
        let mut engine = Engine::default();
        engine.execute(statement).unwrap();
        let x = engine.array("x").unwrap();
        assert_eq!(x.class(), "logical", "{statement}");
        assert!(format!("{x:?}").contains("data: [1]"), "{statement}: {x:?}");
    };
    // Full-size U and V of a tall and a wide matrix are unitary and reproduce A.
    for a in ["[1+1i 2; 3 4-1i; 1i 0]", "[1+1i 2 1i; 3 4-1i 0]"] {
        truth(&format!(
            "A = {a}; [U, S, V] = svd(A); x = norm(U'*U - eye(size(U,1))) < 1e-12 && norm(V'*V - eye(size(V,1))) < 1e-12 && norm(A - U*S*V') < 1e-12;"
        ));
    }
    truth("A = [1 1i 2; 2i -2 4i; 1 0 1]; x = abs(norm(A) - max(svd(A))) == 0 && rank(A) == 2;");
    truth(
        "A = [1+1i 2; 3i 4]; x = norm(pinv(A)*A - eye(2)) < 1e-12 && norm(A*pinv(A) - eye(2)) < 1e-12;",
    );
    truth(
        "A = [1i 0.5; 0 -1i]; x = norm(expm(A) - [exp(1i) 0.5*sinh(1i)/1i; 0 exp(-1i)]) < 1e-12;",
    );
    truth("x = norm(expm([3i 2; -1 5+1i]) * expm(-[3i 2; -1 5+1i]) - eye(2)) < 1e-9;");
    truth("x = norm([3+4i 1i -2], -1) == 1/(1/5 + 1 + 1/2) && norm([1i NaN]) ~= norm([1i NaN]);");
    for (statement, message) in [
        (
            "x = svd([1i NaN]);",
            "Input to SVD must not contain NaN or Inf.",
        ),
        ("x = norm([1i Inf; 1 1], 2);", ""),
        (
            "x = cond([1i 2 3], 1);",
            "A is rectangular.  Use the 2 norm.",
        ),
        (
            "[a, b] = svd([1i 2]);",
            "svd does not support 2 outputs; use 1 or 3",
        ),
        ("x = expm([1i 2]);", "Expected input to be a square matrix."),
    ] {
        let mut engine = Engine::default();
        let result = engine.execute(statement);
        if message.is_empty() {
            assert!(result.is_ok(), "{statement}");
            assert_eq!(engine.array("x").unwrap().len(), 1);
        } else {
            let error = result.unwrap_err();
            assert!(error.contains(message), "{statement}: {error}");
        }
    }
}

#[test]
fn complex_qr_survives_underflowing_squares_and_bounds_the_permutation() {
    // |1e-200|^2 underflows to 0: the column norm must come from the scaled form.
    let mut engine = Engine::default();
    engine
        .execute(
            "A = complex([0; 1e-200]); [Q, R] = qr(A); \
             x = max(abs(Q*R - A)) < 1e-210 && abs(R(1, 1)) > 5e-201;",
        )
        .unwrap();
    let x = format!("{:?}", engine.array("x").unwrap());
    assert!(x.contains("Logical") && x.contains("data: [1]"), "{x}");
    // The 3-output permutation is part of the preflight: n*n doubles are
    // refused before anything is allocated.
    let mut engine = Engine::default();
    let error = engine
        .execute("[Q, R, E] = qr(complex(ones(1, 100000)));")
        .unwrap_err();
    eprintln!("PREFLIGHT: {error}");
    assert!(!error.is_empty(), "{error}");
}

#[test]
fn complex_svd_family_survives_values_near_realmax() {
    let truth = |statement: &str| {
        let mut engine = Engine::default();
        engine.execute(statement).unwrap();
        let x = engine.array("x").unwrap();
        assert_eq!(x.class(), "logical", "{statement}");
        assert!(format!("{x:?}").contains("data: [1]"), "{statement}: {x:?}");
    };
    // nalgebra's own scale (max(|re|+|im|)) is Inf here and used to panic. Its
    // Householder step squares the column tail, so a component below about
    // 1e-162 of the largest one is lost (the real path does the same:
    // svd([1 0; 0 1e-300]) is [1; 0]); s(2) is 0 or 1 here, the reference gives 1.
    truth(
        "s = svd([1e308+1e308i 0; 0 1]); x = abs(s(1)/1.4142135623730951e308 - 1) < 1e-14 && s(2) >= 0 && s(2) <= 1;",
    );
    // Reference behavior: tol = 2*eps(1.4e308) is far above 1, so the rank is 1.
    truth(
        "A = [1e308+1e308i 0; 0 1]; x = abs(norm(A)/1.4142135623730951e308 - 1) < 1e-14 && rank(A) == 1;",
    );
    // A second singular value that survives is scaled back as well.
    truth(
        "s = svd([1e308+1e308i 0; 0 1e200]); x = abs(s(1)/1.4142135623730951e308 - 1) < 1e-14 && abs(s(2)/1e200 - 1) < 1e-14;",
    );
    truth(
        "A = [1e308+1e308i 0; 0 1e200]; x = abs(norm(A)/1.4142135623730951e308 - 1) < 1e-14 && abs(cond(A)/1.4142135623730951e108 - 1) < 1e-14 && rank(A) == 1 && rank(A, 1e100) == 2;",
    );
    truth(
        "A = [1e308+1e308i 0; 0 1e200]; [U, S, V] = svd(A); x = norm(U'*U - eye(2)) < 1e-12 && norm(V'*V - eye(2)) < 1e-12 && abs(S(1,1)/1.4142135623730951e308 - 1) < 1e-14 && abs(S(2,2)/1e200 - 1) < 1e-14;",
    );
    truth(
        "A = [1e308+1e308i 0; 0 1e200]; N = null(A); O = orth(A); x = size(N, 2) == 1 && size(O, 2) == 1 && abs(N(1)) < 1e-12 && abs(abs(N(2)) - 1) < 1e-12;",
    );
    // Tiny components are scaled up exactly.
    truth(
        "s = svd([1e-310i 0; 0 1e-310]); x = abs(s(1)/1e-310 - 1) < 1e-9 && abs(s(2)/1e-310 - 1) < 1e-9;",
    );
}
