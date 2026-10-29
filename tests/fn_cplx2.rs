//! Reference cplx2 corpus (complex input to sum, prod, mean, cumsum,
//! cumprod, diff, std, var, median, mode, sort, sortrows, unique, max, min,
//! round, floor, ceil, fix, flip*, rot90, circshift, cat, repmat, reshape, nnz,
//! cross, dot, plus narrowing and display checks), stored as expected values. The LONG flags
//! come from the case list `fixtures/fn/cplx2-cases.txt`; the captured fixture headers omit them. No reference
//! is contacted by this test. See docs/tasks/2026-10-25-complex-corpora-report.md.
use matrust::Engine;

const REFERENCE: &str = include_str!("fixtures/fn/cplx2-expected.txt");
const CASE_LIST: &str = include_str!("fixtures/fn/cplx2-cases.txt");
const TOTAL: usize = 174;

/// Cases that cannot pass for a reason outside this task, with the reason.
/// Each one still runs; a KNOWN case that starts to pass is reported as stale.
const KNOWN: &[(usize, &str)] = &[(
    153,
    "Indexing: y(1) of a complex array with zero imaginary parts stays complex (isreal 0); the reference narrows indexing results to real. Indexing code is outside this task.",
)];

/// Comparison rules other than byte-exact console text.
const ERROR_MESSAGE: &str = "Compare Diagnostic.message exactly, excluding the engine's line/column wrapper; the oracle captured MException.message.";
const WARNING_LINE: &str = "Compare the [Warning: ...] text exactly without its backtrace (it names the oracle script and folder), then the rest of the console text exactly.";
const HARNESS: &[(usize, &str)] = &[
    (111, WARNING_LINE),
    (114, WARNING_LINE),
    (117, ERROR_MESSAGE),
    (128, ERROR_MESSAGE),
    (135, ERROR_MESSAGE),
    (140, ERROR_MESSAGE),
    (170, ERROR_MESSAGE),
    (171, ERROR_MESSAGE),
    (172, ERROR_MESSAGE),
    (173, ERROR_MESSAGE),
    (174, ERROR_MESSAGE),
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
        Some(r) if r == WARNING_LINE => actual == warning_only(expected),
        _ => actual == expected,
    }
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
        assert_eq!(
            case.output.contains("[\u{8}Warning: "),
            rule(case.id) == Some(WARNING_LINE),
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
fn warning_rule_drops_only_the_backtrace() {
    assert_eq!(
        warning_only("[\u{8}Warning: a.]\u{8} \n[\u{8}> In <a>x</a>\n  In y]\u{8} \n\nx =\n"),
        "Warning: a.\n\nx =\n"
    );
}

#[test]
fn cplx2_matches_reference() {
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

fn complex_data(engine: &Engine, name: &str) -> Vec<(f64, f64)> {
    let array = engine.array(name).unwrap();
    let z = array.as_complex().unwrap();
    z.data().iter().map(|v| (v.re, v.im)).collect()
}

#[test]
fn shape_functions_keep_signed_zero_imaginary_parts() {
    let mut e = Engine::default();
    e.execute("z = complex([1 2 3], [-0 0 -0]); a = fliplr(z); b = flip(z, 2); c = circshift(z, [0 1]); d = reshape(z, 3, 1); s = sort(z, 'descend'); f = flipud(z.');")
        .unwrap();
    let signs = |name: &str| -> Vec<bool> {
        complex_data(&e, name)
            .iter()
            .map(|v| v.1.is_sign_negative())
            .collect()
    };
    assert_eq!(signs("a"), [true, false, true]);
    assert_eq!(signs("b"), [true, false, true]);
    assert_eq!(signs("c"), [true, true, false]);
    assert_eq!(signs("d"), [true, false, true]);
    assert_eq!(signs("s"), [true, false, true]);
    assert_eq!(signs("f"), [true, false, true]);
}

#[test]
fn complex_sums_match_the_real_path_bit_for_bit() {
    let mut e = Engine::default();
    e.execute("x = sin(1:100000); y = cos(1:100000) / 3; z = complex(x, y); s = sum(z); m = mean(z); r = [sum(x) sum(y) mean(x) mean(y)]; ok = double(isequal(real(cumsum(z)), cumsum(x)) && isequal(imag(cumsum(z)), cumsum(y)));")
        .unwrap();
    let real =
        |name: &str| -> Vec<f64> { e.array(name).unwrap().as_numeric().unwrap().data().to_vec() };
    let r = real("r");
    assert_eq!(complex_data(&e, "s"), [(r[0], r[1])]);
    assert_eq!(complex_data(&e, "m"), [(r[2], r[3])]);
    assert_eq!(real("ok"), [1.]);
}

#[test]
fn unique_mode_and_sort_options() {
    let mut e = Engine::default();
    e.execute("[u, i, j] = unique([2 1i 2 1i 3], 'last'); [v, k] = unique([3 1i 3 2], 'stable'); [m, f] = mode([1i 2; 1i 2; 3 2]); [s, n] = sort([1i 2], 3); w = unique([1i 2; 1i 2; NaN 1], 'rows');")
        .unwrap();
    let real =
        |name: &str| -> Vec<f64> { e.array(name).unwrap().as_numeric().unwrap().data().to_vec() };
    assert_eq!(real("i"), [4., 3., 5.]);
    assert_eq!(real("j"), [2., 1., 2., 1., 3.]);
    assert_eq!(complex_data(&e, "v"), [(3., 0.), (0., 1.), (2., 0.)]);
    assert_eq!(real("k"), [1., 2., 4.]);
    assert_eq!(complex_data(&e, "m"), [(0., 1.), (2., 0.)]);
    assert_eq!(real("f"), [2., 3.]);
    assert_eq!(complex_data(&e, "s"), [(0., 1.), (2., 0.)]);
    assert_eq!(real("n"), [1., 1.]);
    let w = e.array("w").unwrap();
    assert_eq!((w.rows(), w.cols()), (2, 2));
}

#[test]
fn negative_zero_imaginary_part_prints_with_a_plus_sign() {
    let echo = |setup: &str, statement: &str| -> String {
        let mut e = Engine::default();
        e.execute(setup).unwrap();
        let result = e.execute(statement).unwrap();
        result.output.join("\n")
    };
    for format in ["format short", "format long"] {
        let scalar = echo(format, "x = complex(1.5, -0)");
        assert!(scalar.contains(" + 0.0"), "{format}: {scalar}");
        assert!(!scalar.contains(" - "), "{format}: {scalar}");
        let matrix = echo(format, "x = complex([1.5 2.5], [-0 -0.25])");
        assert!(matrix.contains(" + 0.0"), "{format}: {matrix}");
        assert!(matrix.contains(" - 0.25"), "{format}: {matrix}");
        // A negative imaginary part that only displays as zero keeps its sign.
        let tiny = echo(format, "x = 1.5 - 1e-20i");
        assert!(tiny.contains(" - 0.0"), "{format}: {tiny}");
    }
}
