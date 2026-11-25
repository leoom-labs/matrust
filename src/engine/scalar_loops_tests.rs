//! The scalar fast path must give the general path's bits, class and errors
//! (NaN results compare as NaN: their payload bits are not specified).
//! The general path is forced by indexing every operand (`a(1)`): an indexed
//! leaf is not a plain name, so the whole expression evaluates as before.
use super::super::{Engine, Limits};

/// Operands as MATLAB statements; each is assigned on the general path.
const OPERANDS: &[&str] = &[
    "0", "-0", "1", "-1", "2.5", "-7.25", "NaN", "-NaN", "Inf", "-Inf", "2^53", "2^53 - 1",
    "-(2^53)", "1e308", "realmin", "5e-324", "true", "false",
];

const BINARY: &[&str] = &[
    "+", "-", "*", ".*", "/", "./", "\\", "==", "~=", "<", "<=", ">", ">=", "&", "|", "&&", "||",
];

#[derive(Debug, PartialEq)]
enum Outcome {
    Double(u64),
    Logical(bool),
    Error(String),
}

fn outcome(engine: &mut Engine, statement: &str, name: &str) -> Outcome {
    match engine.execute(statement) {
        Err(message) => Outcome::Error(message),
        Ok(_) => {
            if let Some(m) = engine.logical_variables.get(name) {
                assert_eq!((m.rows(), m.cols()), (1, 1), "{statement}");
                Outcome::Logical(m.data()[0] != 0)
            } else {
                let m = &engine.variables[name];
                assert_eq!((m.rows(), m.cols()), (1, 1), "{statement}");
                let v = m.data()[0];
                // Rust leaves the sign and payload of an arithmetic NaN
                // unspecified (LLVM may commute `a + b`), on both paths.
                Outcome::Double(if v.is_nan() { u64::MAX } else { v.to_bits() })
            }
        }
    }
}

fn engine_with(a: &str, b: &str) -> Engine {
    let mut engine = Engine::with_limits(Limits::local());
    engine.execute(&format!("a = {a}; b = {b};")).unwrap();
    engine
}

#[test]
fn binary_operators_match_the_general_path_bit_for_bit() {
    let mut checked = 0;
    for a in OPERANDS {
        for b in OPERANDS {
            for op in BINARY {
                let mut engine = engine_with(a, b);
                let fast = outcome(&mut engine, &format!("f = a {op} b;"), "f");
                let mut engine = engine_with(a, b);
                let general = outcome(&mut engine, &format!("g = a(1) {op} b(1);"), "g");
                assert_eq!(fast, general, "a = {a}; b = {b}; a {op} b");
                checked += 1;
            }
        }
    }
    assert_eq!(checked, OPERANDS.len() * OPERANDS.len() * BINARY.len());
}

#[test]
fn unary_operators_and_nesting_match_the_general_path() {
    for a in OPERANDS {
        for (fast, general) in [
            ("-a", "-a(1)"),
            ("+a", "+a(1)"),
            ("~a", "~a(1)"),
            ("-(-a)", "-(-a(1))"),
            ("~~a", "~~a(1)"),
            ("2*a+1", "2*a(1)+1"),
            ("(a - 3) / (a + 3) * a", "(a(1) - 3) / (a(1) + 3) * a(1)"),
            (
                "a > 0 & a < 3 | a == -1",
                "a(1) > 0 & a(1) < 3 | a(1) == -1",
            ),
            ("(a > 0) + (a ~= a) * 2", "(a(1) > 0) + (a(1) ~= a(1)) * 2"),
            ("a + -0", "a(1) + -0"),
            ("-0 * a", "-0 * a(1)"),
        ] {
            let mut engine = engine_with(a, "0");
            let f = outcome(&mut engine, &format!("f = {fast};"), "f");
            let mut engine = engine_with(a, "0");
            let g = outcome(&mut engine, &format!("g = {general};"), "g");
            assert_eq!(f, g, "a = {a}; {fast}");
        }
    }
}

#[test]
fn loops_give_exact_sums_and_keep_their_variable() {
    let mut engine = Engine::with_limits(Limits::local());
    engine
        .execute("s = 0; for k = 1:1000; s = s + (2*k + 1); end")
        .unwrap();
    assert_eq!(engine.variables["s"].data()[0], 1_002_000.);
    assert_eq!(engine.variables["k"].data()[0], 1000.);

    // Integers near 2^53 round exactly as on the general path.
    let mut engine = Engine::with_limits(Limits::local());
    engine
        .execute("s = 2^53 - 4; t = s; for k = 1:9; s = s + 1; t = t(1) + 1; end")
        .unwrap();
    let (s, t) = (
        engine.variables["s"].data()[0],
        engine.variables["t"].data()[0],
    );
    assert_eq!(s.to_bits(), t.to_bits());
    // 2^53 + 1 rounds to even, back to 2^53.
    assert_eq!(s, 9_007_199_254_740_992.);

    // Signed zero, NaN and Inf propagate through the in-place store.
    let mut engine = Engine::with_limits(Limits::local());
    engine
        .execute("z = 0; for k = 1:3; z = -z * 0; end; n = 0; for k = 1:2; n = n + NaN; end; i = 1; for k = 1:3; i = i * 1e308; end")
        .unwrap();
    assert_eq!(engine.variables["z"].data()[0].to_bits(), (-0f64).to_bits());
    assert!(engine.variables["n"].data()[0].is_nan());
    assert_eq!(engine.variables["i"].data()[0], f64::INFINITY);

    // while and if conditions on scalars.
    let mut engine = Engine::with_limits(Limits::local());
    engine
        .execute("n = 0; c = 0; while n < 100; n = n + 1; if n > 90 && n ~= 95, c = c + 1; elseif n == 1, c = c + 100; end; end")
        .unwrap();
    assert_eq!(engine.variables["n"].data()[0], 100.);
    assert_eq!(engine.variables["c"].data()[0], 109.);
    // NaN is a true condition, as on the general path.
    engine.execute("r = 0; if NaN + 0, r = 1; end").unwrap();
    assert_eq!(engine.variables["r"].data()[0], 1.);
}

#[test]
fn other_classes_keep_their_class_and_saturation() {
    let mut engine = Engine::with_limits(Limits::local());
    engine
        .execute(concat!(
            "x = int8(100); for k = 1:10; x = x + k; end; ",
            "okInt = isa(x, 'int8') && x == 127; ",
            "u = uint8(3); for k = 1:5; u = u - 1; end; ",
            "okUint = isa(u, 'uint8') && u == 0; ",
            "y = single(1); for k = 1:3; y = y + 0.1; end; ",
            "okSingle = isa(y, 'single') && y == single(1) + single(0.1) + single(0.1) + single(0.1); ",
            "c = 'a'; d = c + 1; okChar = isa(d, 'double') && d == 98; ",
            "t = true; v = t + t; w = t & false; okLogical = isa(v, 'double') && v == 2 && islogical(w) && ~w; ",
            "m = -true; okNeg = isa(m, 'double') && m == -1; ",
            "q = 1 < 2; okCompare = islogical(q) && q; ",
            "p = 3; p = p > 2; okRetype = islogical(p) && p; ",
            "e = 2; e = e + 1i; okComplex = ~isreal(e);"
        ))
        .unwrap();
    for name in [
        "okInt",
        "okUint",
        "okSingle",
        "okChar",
        "okLogical",
        "okNeg",
        "okCompare",
        "okRetype",
        "okComplex",
    ] {
        let value = &engine.logical_variables[name];
        assert!(value.data()[0] != 0, "{name}");
    }
}

#[test]
fn shared_values_are_not_written_through() {
    let mut engine = Engine::with_limits(Limits::local());
    engine
        .execute("a = 1; b = a; f = @() a; for k = 1:3; a = a + 1; end; r = f();")
        .unwrap();
    assert_eq!(engine.variables["a"].data()[0], 4.);
    assert_eq!(engine.variables["b"].data()[0], 1.);
    assert_eq!(engine.variables["r"].data()[0], 1.);
    // A workspace snapshot (as the desktop takes one) keeps its value.
    let snapshot = engine.variables.clone();
    engine.execute("a = a * 10;").unwrap();
    assert_eq!(snapshot["a"].data()[0], 4.);
    assert_eq!(engine.variables["a"].data()[0], 40.);
}

#[test]
fn errors_and_budgets_are_unchanged() {
    let mut engine = Engine::with_limits(Limits::local());
    engine.execute("a = NaN; b = 1;").unwrap();
    let fast = engine.execute("c = a && b;").unwrap_err();
    let general = engine.execute("c = a(1) && b(1);").unwrap_err();
    assert_eq!(fast, general);
    assert!(!engine.variables.contains_key("c"));

    // One evaluation unit per node, as Engine::eval charges.
    let mut limits = Limits::local();
    limits.evaluation_units = Some(5);
    let mut engine = Engine::with_limits(limits);
    engine.execute("a = 1; b = a + a + a;").unwrap();
    let error = engine.execute("b = a + a + a + a;").unwrap_err();
    assert!(error.contains("evaluation budget"), "{error}");
    assert_eq!(engine.variables["b"].data()[0], 3.);

    // Undefined names still fail on the general path.
    let error = engine.execute("z = a + undefinedThing;").unwrap_err();
    assert!(error.contains("undefinedThing"), "{error}");
}

#[test]
fn step_limits_and_cancellation_stop_fast_loops() {
    let mut limits = Limits::local();
    limits.steps = Some(10_000);
    let mut engine = Engine::with_limits(limits);
    let error = engine
        .execute("s = 0; for k = 1:1e6; s = s + (2*k + 1); end")
        .unwrap_err();
    assert!(error.contains("10000 statement/loop steps"), "{error}");

    let mut engine = Engine::with_limits(Limits::local());
    let cancel = engine.cancel.clone();
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    let error = engine.execute("s = 0; s = s + 1;").unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
}

#[test]
fn workspace_queries_see_fast_assignments() {
    let mut engine = Engine::with_limits(Limits::local());
    engine
        .execute("x = 1; for k = 1:3; x = x * 2; end; e = exist('x'); clear x; g = exist('x');")
        .unwrap();
    assert_eq!(engine.variables["e"].data()[0], 1.);
    assert_eq!(engine.variables["g"].data()[0], 0.);
    let before = engine.workspace_bytes();
    engine.execute("y = 1; y = y + 1;").unwrap();
    let after = engine.workspace_bytes();
    engine.execute("z = 2;").unwrap();
    assert_eq!(after - before, engine.workspace_bytes() - after);
}
