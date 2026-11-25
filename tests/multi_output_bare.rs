use matrust::Engine;

fn assert_pair(engine: &Engine, first: f64, second: f64) {
    assert_eq!(engine.variables["x"].data(), &[first]);
    assert_eq!(engine.variables["y"].data(), &[second]);
}

#[test]
fn calls_a_zero_argument_function_on_the_search_path() {
    let dir = tempfile::tempdir().unwrap();
    let functions = dir.path().join("functions");
    std::fs::create_dir(&functions).unwrap();
    std::fs::write(functions.join("two.m"), "function [a,b]=two, a=1; b=2; end").unwrap();
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    engine.set_file_directory(dir.path()).unwrap();
    engine
        .execute("addpath('functions'); [x,y] = two;")
        .unwrap();
    assert_pair(&engine, 1., 2.);

    engine
        .execute("slots = cell(1,2); [slots{:}] = two; clear x y; [x,y] = slots{:};")
        .unwrap();
    assert_pair(&engine, 1., 2.);
}

#[test]
fn calls_a_local_function_of_a_script_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("run_local.m"),
        "[x,y] = two;\nfunction [a,b] = two\na = 3; b = 4;\nend\n",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    engine.execute("run_local;").unwrap();
    assert_pair(&engine, 3., 4.);
}

#[test]
fn nargout_counts_ignored_targets() {
    let mut engine = Engine::default();
    let result = engine
        .execute(
            "[x,~] = two3;\nfunction [a,b,c] = two3\n\
             fprintf('nargout=%d\\n', nargout);\na = 1; b = 2; c = 3;\nend",
        )
        .unwrap();
    assert_eq!(result.output, ["nargout=2"]);
    assert_eq!(engine.variables["x"].data(), &[1.]);
}

#[test]
fn dispatches_builtins_with_zero_arguments() {
    let mut engine = Engine::default();
    engine
        .execute("lastwarn('sample','Test:sample'); [msg,id] = lastwarn;")
        .unwrap();
    assert_eq!(engine.array("msg").unwrap().preview(32), "'sample'");
    assert_eq!(engine.array("id").unwrap().preview(32), "'Test:sample'");

    let error = engine.execute("[x,y] = deal;").unwrap_err();
    assert!(error.contains("Not enough input arguments."), "{error}");
}

#[test]
fn bare_variables_shadow_functions_and_reject_multiple_outputs() {
    let mut engine = Engine::default();
    engine
        .execute("two = 3; h = @() error('Handle must not run'); [x] = two; [copy] = h; y = 4;")
        .unwrap();
    assert!(
        engine
            .array("copy")
            .unwrap()
            .same_snapshot(&engine.array("h").unwrap())
    );
    for source in [
        "[x,y] = two;\nfunction [a,b] = two\na = 1; b = 2;\nend",
        "[x,y] = h;",
    ] {
        let error = engine.execute(source).unwrap_err();
        assert!(error.contains("Too many output arguments."), "{error}");
        assert_pair(&engine, 3., 4.);
    }
}

#[test]
fn assigns_cell_and_struct_lists_in_order() {
    let mut engine = Engine::default();
    engine
        .execute("c = {1,2}; s = struct('f',{3,4}); many = {5,6,7};")
        .unwrap();
    for (source, first, second) in [
        ("[x,y] = c{:};", 1., 2.),
        ("[x,y] = c{1:2};", 1., 2.),
        ("[x,y] = s.f;", 3., 4.),
        ("[x,y] = many{:};", 5., 6.),
        ("[x,~,y] = many{:};", 5., 7.),
        ("[x,y] = many{[3 1]};", 7., 5.),
    ] {
        engine.execute(source).unwrap();
        assert_pair(&engine, first, second);
    }
}

#[test]
fn rejects_short_lists_before_assigning_any_targets() {
    let mut engine = Engine::default();
    engine
        .execute("x = 5; y = 6; c = {1}; s = struct('f',2); empty = {};")
        .unwrap();
    for source in ["[x,y] = c{1};", "[x,y] = s.f;", "[x,y] = empty{:};"] {
        let error = engine.execute(source).unwrap_err();
        assert!(
            error.contains("Insufficient number of outputs from right hand side of equal sign to satisfy assignment."),
            "{source}: {error}"
        );
        assert_pair(&engine, 5., 6.);
    }
}
