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

fn assert_err_id(engine: &mut Engine, code: &str, expected_id: &str, expected_msg_part: &str) {
    let err = engine.execute(code).unwrap_err();
    assert!(
        err.contains(expected_msg_part),
        "expected '{expected_msg_part}' in error: {err}"
    );
    engine
        .execute(&format!(
            "try, {code}; catch e, id__ = e.identifier; msg__ = e.message; end"
        ))
        .unwrap();
    let id = engine.array("id__").unwrap().preview(200);
    let msg = engine.array("msg__").unwrap().preview(200);
    assert!(
        id.contains(expected_id),
        "expected id '{expected_id}', got '{id}' (msg: {msg})"
    );
    assert!(
        msg.replace("''", "'").contains(expected_msg_part),
        "expected msg '{expected_msg_part}', got '{msg}'"
    );
}

#[test]
fn arguments_block_core_af_and_nargin() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("af.m"),
        "\
function [y, n] = af(x, s)
    % Leading comment
    arguments
        x (1,:) double {mustBeNonnegative}
        % Default uses earlier arg
        s (1,1) double = ...
            numel(x)
    end
    y = x * s;
    n = nargin;
end
",
    )
    .unwrap();

    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();

    // af([1 2]) with default s = numel(x) = 2 -> [2 4], nargin == 1
    engine.execute("[y, n] = af([1 2]);").unwrap();
    assert_eq!(
        engine.array("y").unwrap().as_numeric().unwrap().data(),
        &[2.0, 4.0]
    );
    assert_eq!(
        engine.array("n").unwrap().as_numeric().unwrap().data(),
        &[1.0]
    );

    // af(int8([1 2]), 3) -> double [3 6], nargin == 2
    engine
        .execute("[y, n] = af(int8([1 2]), 3); c = class(y);")
        .unwrap();
    assert_eq!(
        engine.array("y").unwrap().as_numeric().unwrap().data(),
        &[3.0, 6.0]
    );
    assert_eq!(
        engine.array("n").unwrap().as_numeric().unwrap().data(),
        &[2.0]
    );
    assert!(engine.array("c").unwrap().preview(32).contains("double"));

    // af(-1) -> mustBeNonnegative error with position
    assert_err_id(
        &mut engine,
        "af(-1)",
        "MATLAB:validators:mustBeNonnegative",
        "Invalid argument at position 1. Value must be nonnegative.",
    );

    // A column vector is converted to the declared row shape.
    engine.execute("y = af([1; 2]);").unwrap();
    let y = engine.array("y").unwrap();
    assert_eq!(&*y.dims(), &[1, 2]);
    assert_eq!(y.as_numeric().unwrap().data(), &[2.0, 4.0]);

    // A matrix cannot be converted to a row vector.
    assert_err_id(
        &mut engine,
        "af([1 2; 3 4])",
        "MATLAB:validation:IncompatibleSize",
        "Invalid argument at position 1.",
    );
}

#[test]
fn arguments_block_simple_default_and_scalar_expansion() {
    let mut engine = Engine::default();
    let res = eval_double(
        &mut engine,
        "af([1 2])\nfunction y = af(x, s)\narguments; x (1,:) double {mustBeNonnegative}; s (1,1) double = 1; end\ny = x * s;\nend",
    );
    assert_eq!(res, vec![1.0, 2.0]);

    // Scalar expansion: (2, 3) double given scalar 5 -> 2x3 of 5s
    engine
        .execute(
            "m = expand_fn(5);\nfunction y = expand_fn(x)\narguments\nx (2,3) double\nend\ny = x;\nend",
        )
        .unwrap();
    let m = engine.array("m").unwrap();
    assert_eq!(&*m.dims(), &[2, 3]);
    assert_eq!(m.as_numeric().unwrap().data(), &[5.0; 6]);
}

#[test]
fn arguments_input_and_output_blocks_and_validators_with_args() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("bounded.m"),
        "\
function y = bounded(a, b)
arguments (Input)
  a (1,1) double {mustBeInRange(a, 0, 10)}
  b (1,1) double {mustBeGreaterThan(b, a)}
end
arguments (Output)
  y (1,1) double {mustBePositive}
end
y = b - a;
end
",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("bad_out.m"),
        "\
function y = bad_out(x)
arguments (Output)
  y (1,1) double {mustBePositive}
end
y = x;
end
",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    engine.execute("r = bounded(3, 5);").unwrap();
    assert_eq!(
        engine.array("r").unwrap().as_numeric().unwrap().data(),
        &[2.0]
    );

    assert_err_id(
        &mut engine,
        "bounded(5, 3)",
        "MATLAB:validators:mustBeGreaterThan",
        "Invalid argument at position 2.",
    );
    assert_err_id(
        &mut engine,
        "bad_out(-1)",
        "MATLAB:validators:mustBePositive",
        "Invalid output 'y'.",
    );
}

#[test]
fn arguments_in_classdef_method() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Scaler.m"),
        "\
classdef Scaler
    properties
        Factor = 1
    end
    methods
        function obj = Scaler(f)
            arguments
                f (1,1) double {mustBePositive}
            end
            obj.Factor = f;
        end
        function y = apply(obj, x)
            arguments
                obj
                x (1,:) double {mustBeFinite}
            end
            y = obj.Factor * x;
        end
    end
end
",
    )
    .unwrap();

    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    assert_eq!(
        eval_double(&mut engine, "s = Scaler(3); s.apply([1 2 4])"),
        vec![3.0, 6.0, 12.0]
    );
    assert_err_id(
        &mut engine,
        "Scaler(-2)",
        "MATLAB:validators:mustBePositive",
        "Invalid argument at position 1.",
    );
}

#[test]
fn arguments_as_variable_name_still_works() {
    let mut engine = Engine::default();
    assert_eq!(
        eval_double(
            &mut engine,
            "arguments = [10 20]; arguments(2) = 30; sum(arguments)"
        ),
        vec![40.0]
    );
    assert_eq!(
        eval_double(
            &mut engine,
            "use_var(5)\nfunction y = use_var(x)\narguments = x + 1;\ny = arguments * 2;\nend"
        ),
        vec![12.0]
    );
}

#[test]
fn class_conversions_in_arguments_block() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("need_struct.m"),
        "\
function y = need_struct(s)
arguments
  s struct
end
y = s;
end
",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    engine
        .execute(
            "t = conv_test(1, 'hi', int32(4)); c1 = t{1}; c2 = t{2}; c3 = t{3};\nfunction out = conv_test(l, s, d)\narguments\n  l (1,1) logical\n  s (1,1) string\n  d (1,1) single\nend\nout = {class(l), class(s), class(d)};\nend",
        )
        .unwrap();
    let c1 = engine.array("c1").unwrap().preview(32);
    let c2 = engine.array("c2").unwrap().preview(32);
    let c3 = engine.array("c3").unwrap().preview(32);
    assert!(
        c1.contains("logical") && c2.contains("string") && c3.contains("single"),
        "{c1} {c2} {c3}"
    );

    // Unconvertible value to struct
    assert_err_id(
        &mut engine,
        "need_struct(5)",
        "MATLAB:validation:UnableToConvert",
        "Unable to convert value to struct.",
    );
}

#[test]
fn all_mustbe_validators_directly_callable() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("sample.txt");
    std::fs::write(&file_path, "ok").unwrap();
    let sub_dir = dir.path().join("sub");
    std::fs::create_dir(&sub_dir).unwrap();

    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    let file_str = file_path.to_string_lossy().replace('\'', "''");
    let dir_str = sub_dir.to_string_lossy().replace('\'', "''");
    let must_file_pass = format!("mustBeFile('{file_str}')");
    let must_folder_pass = format!("mustBeFolder('{dir_str}')");

    let cases: &[(&str, &str, &str)] = &[
        (
            "mustBePositive([1 2])",
            "mustBePositive([1 0])",
            "MATLAB:validators:mustBePositive",
        ),
        (
            "mustBeNonnegative([0 2])",
            "mustBeNonnegative([0 -1])",
            "MATLAB:validators:mustBeNonnegative",
        ),
        (
            "mustBeNegative([-1 -2])",
            "mustBeNegative([-1 0])",
            "MATLAB:validators:mustBeNegative",
        ),
        (
            "mustBeNonpositive([0 -2])",
            "mustBeNonpositive([0 1])",
            "MATLAB:validators:mustBeNonpositive",
        ),
        (
            "mustBeNonzero([1 -1])",
            "mustBeNonzero([1 0])",
            "MATLAB:validators:mustBeNonzero",
        ),
        (
            "mustBeFinite([1 -2])",
            "mustBeFinite([1 Inf])",
            "MATLAB:validators:mustBeFinite",
        ),
        (
            "mustBeNonNan([1 Inf])",
            "mustBeNonNan([1 NaN])",
            "MATLAB:validators:mustBeNonNan",
        ),
        (
            "mustBeInteger([1 -2])",
            "mustBeInteger([1 1.5])",
            "MATLAB:validators:mustBeInteger",
        ),
        (
            "mustBeReal([1 2])",
            "mustBeReal(1+2i)",
            "MATLAB:validators:mustBeReal",
        ),
        (
            "mustBeNumeric(int8(1))",
            "mustBeNumeric(true)",
            "MATLAB:validators:mustBeNumeric",
        ),
        (
            "mustBeNumericOrLogical(true)",
            "mustBeNumericOrLogical('a')",
            "MATLAB:validators:mustBeNumericOrLogical",
        ),
        (
            "mustBeNonempty([1])",
            "mustBeNonempty([])",
            "MATLAB:validators:mustBeNonempty",
        ),
        (
            "mustBeMember(2, [1 2 3])",
            "mustBeMember(4, [1 2 3])",
            "MATLAB:validators:mustBeMember",
        ),
        (
            "mustBeInRange(0.5, 0, 1)",
            "mustBeInRange(1, 0, 1, 'exclusive')",
            "MATLAB:validators:mustBeInRange",
        ),
        (
            "mustBeGreaterThan(2, 1)",
            "mustBeGreaterThan(1, 1)",
            "MATLAB:validators:mustBeGreaterThan",
        ),
        (
            "mustBeGreaterThanOrEqual(1, 1)",
            "mustBeGreaterThanOrEqual(0, 1)",
            "MATLAB:validators:mustBeGreaterThanOrEqual",
        ),
        (
            "mustBeLessThan(1, 2)",
            "mustBeLessThan(2, 2)",
            "MATLAB:validators:mustBeLessThan",
        ),
        (
            "mustBeLessThanOrEqual(2, 2)",
            "mustBeLessThanOrEqual(3, 2)",
            "MATLAB:validators:mustBeLessThanOrEqual",
        ),
        (
            "mustBeText([\"a\", \"b\"])",
            "mustBeText(123)",
            "MATLAB:validators:mustBeText",
        ),
        (
            "mustBeTextScalar(\"hi\")",
            "mustBeTextScalar([\"a\", \"b\"])",
            "MATLAB:validators:mustBeTextScalar",
        ),
        (
            "mustBeNonzeroLengthText(\"a\")",
            "mustBeNonzeroLengthText(\"\")",
            "MATLAB:validators:mustBeNonzeroLengthText",
        ),
        (
            must_file_pass.as_str(),
            "mustBeFile('missing.txt')",
            "MATLAB:validators:mustBeFile",
        ),
        (
            must_folder_pass.as_str(),
            "mustBeFolder('missing_dir')",
            "MATLAB:validators:mustBeFolder",
        ),
        (
            "mustBeVector([1 2 3])",
            "mustBeVector(ones(2,2))",
            "MATLAB:validators:mustBeVector",
        ),
        (
            "mustBeRow([1 2 3])",
            "mustBeRow([1; 2])",
            "MATLAB:validators:mustBeRow",
        ),
        (
            "mustBeColumn([1; 2])",
            "mustBeColumn([1 2])",
            "MATLAB:validators:mustBeColumn",
        ),
        (
            "mustBeScalarOrEmpty([])",
            "mustBeScalarOrEmpty([1 2])",
            "MATLAB:validators:mustBeScalarOrEmpty",
        ),
        (
            "mustBeA(int8(1), 'numeric')",
            "mustBeA('hi', 'numeric')",
            "MATLAB:validators:mustBeA",
        ),
        (
            "mustBeUnderlyingType(int8(1), 'int8')",
            "mustBeUnderlyingType(int8(1), 'double')",
            "MATLAB:validators:mustBeUnderlyingType",
        ),
    ];

    for (pass_code, fail_code, err_id) in cases {
        engine
            .execute(pass_code)
            .unwrap_or_else(|e| panic!("pass case failed: {pass_code}: {e}"));
        assert_err_id(&mut engine, fail_code, err_id, "");
    }
}
