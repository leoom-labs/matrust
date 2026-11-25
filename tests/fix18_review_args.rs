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
fn fix18_review_args_command_syntax_end_global_persistent() {
    let mut engine = Engine::default();
    assert_eq!(
        eval_double(&mut engine, "x = 3; function f; end; x -1"),
        vec![2.0]
    );

    let mut engine = Engine::default();
    assert_eq!(eval_double(&mut engine, "global x; x = 3; x -1"), vec![2.0]);

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("pers.m"),
        "\
function y = pers()
    persistent p
    p = 3;
    y = p -1;
end
",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    assert_eq!(eval_double(&mut engine, "pers()"), vec![2.0]);
}

#[test]
fn fix18_review_args_param_named_arguments() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("f.m"),
        "\
function n = f(arguments)
    arguments;
    n = arguments;
end
",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    assert_eq!(eval_double(&mut engine, "f(2)"), vec![2.0]);
}

#[test]
fn fix18_review_args_handle_nv_and_repeating() {
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
    std::fs::write(
        dir.path().join("rep.m"),
        "\
function total = rep(base, x, y)
    arguments
        base (1,1) double
    end
    arguments (Repeating)
        x (1,:) double
        y (1,1) double
    end
    total = base;
    for i = 1:numel(x)
        total = total + sum(x{i}) * y{i};
    end
end
",
    )
    .unwrap();

    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    assert_eq!(
        eval_double(&mut engine, "h = @af; h([1 2], 'Scale', 3)"),
        vec![3.0, 6.0]
    );
    assert_eq!(
        eval_double(&mut engine, "h = @af; h([1 2], Scale=3)"),
        vec![3.0, 6.0]
    );
    assert_eq!(
        eval_double(&mut engine, "hr = @rep; hr(10, [1 2], 3)"),
        vec![19.0]
    );
}

#[test]
fn fix18_review_args_nv_without_optional_positional() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("f.m"),
        "\
function y = f(x, opts)
    arguments
        x double = 2
        opts.Scale = 1
    end
    y = x * opts.Scale;
end
",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    assert_eq!(eval_double(&mut engine, "f(Scale=3)"), vec![6.0]);
}

#[test]
fn fix18_review_args_nv_validator_uses_bound_field() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("f.m"),
        "\
function y = f(opts)
    arguments
        opts.Scale {mustBeInRange(opts.Scale, 1, 5)} = 2
    end
    y = opts.Scale;
end
",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    assert_eq!(eval_double(&mut engine, "f(Scale=3)"), vec![3.0]);
    assert_err(
        &mut engine,
        "f(Scale=10)",
        "Invalid value for name-value argument 'Scale'.",
    );
}

#[test]
fn fix18_review_args_nargin_excludes_nv() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("f.m"),
        "\
function n = f(x, opts)
    arguments
        x = 0
        opts.Scale = 1
    end
    n = nargin;
end
",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("g.m"),
        "\
function n = g(r, opts)
    arguments (Repeating)
        r
    end
    arguments
        opts.Scale = 1
    end
    n = nargin;
end
",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    assert_eq!(eval_double(&mut engine, "f(2, 'Scale', 3)"), vec![1.0]);
    assert_eq!(eval_double(&mut engine, "f(2, Scale=3)"), vec![1.0]);
    assert_eq!(eval_double(&mut engine, "g(Scale=3)"), vec![0.0]);
    assert_eq!(eval_double(&mut engine, "g(10, 20, Scale=3)"), vec![2.0]);
}

#[test]
fn fix18_review_args_repeating_zero_size() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("f.m"),
        "\
function sz = f(x, opts)
    arguments (Repeating)
        x
    end
    arguments
        opts.Scale = 1
    end
    sz = size(x);
end
",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    assert_eq!(eval_double(&mut engine, "f(Scale=3)"), vec![1.0, 0.0]);
}

#[test]
fn fix18_review_args_signature_checks_repeating_and_nv() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("bad_rep.m"),
        "\
function y = bad_rep(x)
    arguments (Repeating)
        typo
    end
    y = typo{1};
end
",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("bad_nv.m"),
        "\
function y = bad_nv(x)
    arguments
        o.K = 3
    end
    y = o.K;
end
",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    assert!(engine.execute("bad_rep(1)").is_err());
    assert!(engine.execute("bad_nv()").is_err());
}

#[test]
fn fix18_review_args_no_forward_reference_in_validators() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("fwd.m"),
        "\
function y = fwd(x, z)
    arguments
        x {mustBeLessThan(x, z)}
        z
    end
    y = x + z;
end
",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("bwd.m"),
        "\
function y = bwd(x, z)
    arguments
        x
        z {mustBeGreaterThan(z, x)}
    end
    y = x + z;
end
",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    assert!(engine.execute("fwd(1, 3)").is_err());
    assert_eq!(eval_double(&mut engine, "bwd(1, 3)"), vec![4.0]);
}

#[test]
fn fix18_review_args_output_defaults_and_error_text() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("out_def.m"),
        "\
function y = out_def()
    arguments (Output)
        y = 7
    end
end
",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("out_val.m"),
        "\
function y = out_val(x)
    arguments (Output)
        y {mustBePositive}
    end
    y = x;
end
",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    assert_err(
        &mut engine,
        "out_def()",
        "Output arguments cannot have default values.",
    );
    assert_err(&mut engine, "out_val(-1)", "Invalid output 'y'.");
}
