use matrust::Engine;

fn eval_double(engine: &mut Engine, code: &str) -> Vec<f64> {
    engine.execute(code).unwrap();
    engine
        .array("ans")
        .unwrap()
        .as_numeric()
        .unwrap()
        .data()
        .to_vec()
}

fn assert_err(engine: &mut Engine, code: &str, expected_msg_part: &str) {
    let err = engine.execute(code).unwrap_err();
    assert!(
        err.contains(expected_msg_part),
        "expected '{expected_msg_part}' in error: {err}"
    );
}

#[test]
fn arguments_name_value_basic_and_syntax() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("af.m"),
        "\
function y = af(x, opts)
    arguments
        x (1,:) double
        opts.Scale (1,1) double {mustBePositive} = 1
        opts.Offset (1,1) double = 0
    end
    y = x * opts.Scale + opts.Offset;
end
",
    )
    .unwrap();

    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();

    // Default values
    assert_eq!(eval_double(&mut engine, "af([1 2])"), vec![1.0, 2.0]);
    // 'Name', value syntax
    assert_eq!(
        eval_double(&mut engine, "af([1 2], 'Scale', 3)"),
        vec![3.0, 6.0]
    );
    // Name=value syntax
    assert_eq!(
        eval_double(&mut engine, "af([1 2], Scale=3)"),
        vec![3.0, 6.0]
    );
    assert_eq!(
        eval_double(&mut engine, "af([1 2], Scale=3, Offset=10)"),
        vec![13.0, 16.0]
    );
    // String scalar name
    assert_eq!(
        eval_double(&mut engine, "af([1 2], \"Scale\", 4)"),
        vec![4.0, 8.0]
    );
    // Scalar struct
    assert_eq!(
        eval_double(&mut engine, "s = struct('Scale', 5); af([1 2], s)"),
        vec![5.0, 10.0]
    );
}

#[test]
fn arguments_name_value_matching_and_errors() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("nv.m"),
        "\
function y = nv(x, opts)
    arguments
        x
        opts.Scale (1,1) double = 1
        opts.Shift (1,1) double = 0
        opts.Bias (1,1) double = 0
    end
    y = x * opts.Scale + opts.Shift + opts.Bias;
end
",
    )
    .unwrap();

    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();

    // Case-insensitive exact & unique partial matching
    assert_eq!(
        eval_double(&mut engine, "nv([1 2], 'scale', 2, 'b', 5)"),
        vec![7.0, 9.0]
    );
    assert_eq!(
        eval_double(&mut engine, "nv([1 2], 'SC', 3)"),
        vec![3.0, 6.0]
    );

    // Ambiguous partial match ('S' matches both Scale and Shift)
    assert_err(&mut engine, "nv([1 2], 'S', 2)", "Ambiguous argument name");

    // Unknown name
    assert_err(
        &mut engine,
        "nv([1 2], 'Unknown', 2)",
        "Invalid argument name 'Unknown'",
    );

    // Odd number of name-value arguments
    assert_err(
        &mut engine,
        "nv([1 2], 'Scale')",
        "Name-value arguments must be passed in pairs",
    );

    // Non-text name
    assert_err(
        &mut engine,
        "nv([1 2], 123, 2)",
        "Name-value argument name must be a character vector or string scalar",
    );
}

#[test]
fn arguments_name_value_multiple_structs_and_optional_field() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("multi.m"),
        "\
function [a, b, has_opt] = multi(x, opts1, opts2)
    arguments
        x (1,1) double
        opts1.X (1,1) double = 2
        opts1.Opt (1,1) double
        opts2.Y (1,1) double = 3
    end
    a = x + opts1.X;
    b = x * opts2.Y;
    has_opt = isfield(opts1, 'Opt');
end
",
    )
    .unwrap();

    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();

    engine
        .execute("[a, b, has_opt] = multi(5, 'Y', 4);")
        .unwrap();
    assert_eq!(
        engine.array("a").unwrap().as_numeric().unwrap().data(),
        &[7.0]
    );
    assert_eq!(
        engine.array("b").unwrap().as_numeric().unwrap().data(),
        &[20.0]
    );
    assert!(matches!(
        engine.array("has_opt").unwrap(),
        matrust::Array::Logical(m) if m.data() == [0]
    ));

    engine
        .execute("[a, b, has_opt] = multi(5, Opt=9, X=10);")
        .unwrap();
    assert_eq!(
        engine.array("a").unwrap().as_numeric().unwrap().data(),
        &[15.0]
    );
    assert!(matches!(
        engine.array("has_opt").unwrap(),
        matrust::Array::Logical(m) if m.data() == [1]
    ));
}

#[test]
fn arguments_repeating_pairs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("rep.m"),
        "\
function [total, n] = rep(base, x, y)
    arguments
        base (1,1) double
    end
    arguments (Repeating)
        x (1,:) double {mustBeNonnegative}
        y (1,1) double
    end
    n = numel(x);
    total = base;
    for i = 1:n
        total = total + sum(x{i}) * y{i};
    end
end
",
    )
    .unwrap();

    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();

    // Zero repeating pairs
    engine.execute("[total, n] = rep(10);").unwrap();
    assert_eq!(
        engine.array("total").unwrap().as_numeric().unwrap().data(),
        &[10.0]
    );
    assert_eq!(
        engine.array("n").unwrap().as_numeric().unwrap().data(),
        &[0.0]
    );

    // Two repeating pairs
    engine
        .execute("[total, n] = rep(10, [1 2], 3, int8([4 5]), 2);")
        .unwrap();
    assert_eq!(
        engine.array("total").unwrap().as_numeric().unwrap().data(),
        &[10.0 + 3.0 * 3.0 + 9.0 * 2.0]
    );
    assert_eq!(
        engine.array("n").unwrap().as_numeric().unwrap().data(),
        &[2.0]
    );

    // Incomplete repeating group
    assert_err(
        &mut engine,
        "rep(10, [1 2])",
        "Repeating arguments must be passed in groups of 2",
    );

    // Validation failure in repeating group
    assert_err(
        &mut engine,
        "rep(10, [1 2], 3, [-1 2], 2)",
        "Value must be nonnegative",
    );
}
