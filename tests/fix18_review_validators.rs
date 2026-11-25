use matrust::Engine;

fn class_files(files: &[(&str, &str)]) -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in files {
        std::fs::write(dir.path().join(name), source).unwrap();
    }
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    (dir, engine)
}

fn expect_error(engine: &mut Engine, code: &str, fragments: &[&str]) {
    let error = engine.execute(code).unwrap_err();
    for fragment in fragments {
        assert!(
            error.contains(fragment),
            "{code}: expected {fragment:?} in {error:?}"
        );
    }
}

#[test]
fn fix18_review_validators_implicit_property_defaults() {
    let (_dir, mut engine) = class_files(&[
        (
            "C.m",
            "\
classdef C
  properties
    P (1,1) double
    N (2,3) uint64
    E (1,:) single
    T (2,3) char
    L (2,1) cell
    S (1,2) string
    D (1,2) struct
    U (2,1)
    F (1,2) logical
    W (:,:) uint32
    O (1,1) ChildValue
  end
end",
        ),
        (
            "ChildValue.m",
            "classdef ChildValue\nproperties\nX = 4\nend\nend",
        ),
        ("Derived.m", "classdef Derived < C\nend"),
        (
            "Bad.m",
            "classdef Bad\nproperties\nP (1,1) double {mustBePositive}\nend\nend",
        ),
        ("BadDerived.m", "classdef BadDerived < Bad\nend"),
    ]);
    engine
        .execute(
            r#"
c = C(); d = Derived(); p = c.P; inherited = d.P;
assert(isequal(p, 0) && isequal(inherited, 0));
assert(isa(d.N, 'uint64') && isequal(d.N, uint64(zeros(2,3))));
assert(isa(c.E, 'single') && isequal(size(c.E), [1 0]));
assert(isequal(c.T, ['   '; '   ']));
assert(isequal(c.L, {[]; []}));
assert(isequal(c.S, ["" ""]));
assert(isa(c.D, 'struct') && isequal(size(c.D), [1 2]));
assert(isequal(c.U, [0; 0]));
assert(isequal(c.F, [false false]));
assert(isa(c.W, 'uint32') && isequal(size(c.W), [0 0]));
child = c.O; assert(isa(child, 'ChildValue') && child.X == 4);
"#,
        )
        .unwrap();
    for code in ["Bad();", "BadDerived();", "BadDerived();"] {
        expect_error(
            &mut engine,
            code,
            &["property 'P'", "Value must be positive"],
        );
    }
}

#[test]
fn fix18_review_validators_bracketed_property_assignment_validates_before_writing() {
    let (_dir, mut engine) = class_files(&[(
        "C.m",
        "classdef C\nproperties\nP (1,1) double {mustBeNonnegative} = 0\nend\nend",
    )]);
    engine.execute("c = C(); k = 9;").unwrap();
    expect_error(&mut engine, "[c.P,k]=deal([1 2],1);", &["size (1,1)"]);
    expect_error(&mut engine, "[k,c.P]=deal(1,[1 2]);", &["size (1,1)"]);
    expect_error(&mut engine, "[c.P,k]=deal(-1,1);", &["must be nonnegative"]);
    engine.execute("assert(c.P == 0 && k == 9); [c.P,k]=deal(int8(3),1); assert(isa(c.P,'double') && c.P == 3 && k == 1);").unwrap();
}

#[test]
fn fix18_review_validators_bracketed_property_assignment_runs_setters() {
    let mut engine = Engine::default();
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/classdef2");
    engine.set_function_directory(&fixtures).unwrap();
    engine.execute("c = Temp(10); k = 9;").unwrap();
    expect_error(
        &mut engine,
        "[c.celsius,k]=deal(-300,1);",
        &["below absolute zero"],
    );
    engine.execute("assert(c.celsius == 10 && k == 9); [c.fahrenheit,k]=deal(32,1); assert(c.celsius == 0 && c.sets == 2 && k == 1);").unwrap();
    engine
        .execute(
            "h = Logged(); alias = h; [h.value,k]=deal(5,2); assert(alias.value == 10 && k == 2);",
        )
        .unwrap();
}

#[test]
fn fix18_review_validators_argument_row_conversion_precedes_validators() {
    let mut engine = Engine::default();
    engine.execute("y = f([1;2]);\nfunction y = f(x)\narguments\nx (1,:) double {mustBeRow}\nend\ny = x;\nend").unwrap();
    let y = engine.array("y").unwrap();
    assert_eq!(&*y.dims(), &[1, 2]);
    assert_eq!(y.as_numeric().unwrap().data(), &[1., 2.]);
}

#[test]
fn fix18_review_validators_integer_comparisons_are_exact() {
    let mut engine = Engine::default();
    engine
        .execute("v=bitshift(uint64(1),53); u=v+uint64(1); s=-int64(v); t=s-int64(1);")
        .unwrap();
    expect_error(
        &mut engine,
        "mustBeLessThanOrEqual(v+uint64(1),v);",
        &["less than or equal"],
    );
    expect_error(
        &mut engine,
        "mustBeGreaterThanOrEqual(t,s);",
        &["greater than or equal", "-9007199254740992"],
    );
    expect_error(
        &mut engine,
        "mustBeLessThanOrEqual(u,double(v));",
        &["less than or equal"],
    );
    expect_error(&mut engine, "mustBeInRange(u,v,v);", &["in the range"]);
    engine
        .execute(
            "\
mustBeGreaterThan([u u],v);
mustBeLessThan(t,s);
mustBeLessThan(v,u);
mustBeLessThan(double(v),u);
mustBeGreaterThanOrEqual(v,v);
mustBeLessThanOrEqual(u,u);
mustBeInRange(u,v,u,'exclude-lower');
mustBeGreaterThan(sparse([1 2]),uint64(0));
mustBeGreaterThan(1,sparse(0));
",
        )
        .unwrap();
    expect_error(
        &mut engine,
        "mustBeGreaterThan(sparse([1 0]),uint64(0));",
        &["greater than"],
    );
}

#[test]
fn fix18_review_validators_membership_accepts_strings_and_lists_allowed_values() {
    let mut engine = Engine::default();
    engine
        .execute(
            r#"mustBeMember("red",["red","blue"]); mustBeMember(["blue";"red"],{'red','blue'});"#,
        )
        .unwrap();
    expect_error(
        &mut engine,
        "mustBeMember(3,[1 2]);",
        &["specified set: [1, 2]"],
    );
    expect_error(
        &mut engine,
        r#"mustBeMember("Red",["red","blue"]);"#,
        &["specified set", "\"red\"", "\"blue\""],
    );
}

#[test]
fn fix18_review_validators_text_accepts_character_matrices() {
    let mut engine = Engine::default();
    engine.execute("mustBeText(['ab';'cd']);").unwrap();
    expect_error(&mut engine, "mustBeText(12);", &["character"]);
    expect_error(
        &mut engine,
        "mustBeTextScalar(['ab';'cd']);",
        &["character vector"],
    );
}

#[test]
fn fix18_review_validators_numeric_accepts_empty_nonnumeric_inputs() {
    let mut engine = Engine::default();
    engine
        .execute("mustBeNumeric({}); mustBeNumeric('');")
        .unwrap();
    expect_error(&mut engine, "mustBeNumeric({1});", &["must be numeric"]);
    expect_error(&mut engine, "mustBeNumeric(false);", &["must be numeric"]);
}
