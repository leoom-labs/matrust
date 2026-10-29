use matrust::{Engine, Matrix};

fn seed() -> Engine {
    let mut engine = Engine::default();
    engine
        .execute("A=[1 2 3];B=A;mask=logical([0 1]);keep=77;")
        .unwrap();
    engine
}
fn assert_unchanged(engine: &Engine, before: &std::collections::BTreeMap<String, matrust::Array>) {
    let after = engine.workspace();
    assert_eq!(after.len(), before.len());
    for (name, value) in before {
        assert!(value.same_snapshot(&after[name]), "Changed {name}");
    }
}

#[test]
fn clear_command_and_function_forms_remove_only_named_numeric_and_logical_bindings() {
    for source in [
        "clear A mask",
        "clear('A','mask');",
        "clear \"A\" 'mask' % comment",
        "clear A ... continued\n mask;",
    ] {
        let mut engine = seed();
        let retained = engine.variables["A"].clone();
        let pointer = engine.variables["B"].data().as_ptr();
        let result = engine.execute(source).unwrap();
        assert!(!engine.variables.contains_key("A"));
        assert!(!engine.logical_variables.contains_key("mask"));
        assert_eq!(engine.variables["B"].data().as_ptr(), pointer);
        assert_eq!(retained.data(), &[1., 2., 3.]);
        assert_eq!(engine.variables["keep"].data(), &[77.]);
        assert_eq!(engine.workspace_bytes(), 32);
        assert!(!engine.variables.contains_key("ans"));
        assert!(result.output.is_empty() && !result.clear_console);
    }
}

#[test]
fn clear_missing_duplicate_case_sensitive_and_ans_names_do_not_create_results() {
    let mut engine = seed();
    engine.execute("a=4;ans=8;clear A A missing ans;").unwrap();
    assert_eq!(engine.variables["a"].data(), &[4.]);
    assert!(engine.array("A").is_none() && engine.array("ans").is_none());
    assert!(engine.array("B").is_some());
    engine.execute("clear('sin','notLoaded');").unwrap();
    assert_eq!(engine.variables["keep"].data(), &[77.]);
}

#[test]
fn clear_workspace_forms_preserve_figures_output_and_live_functions() {
    for command in ["clear", "clear()", "clear variables", "clear('variables')"] {
        let mut engine = seed();
        engine
            .execute("plot([1 2 3]);figure(2);plot([3 2]);")
            .unwrap();
        let figures: Vec<_> = engine
            .figures()
            .map(|(id, figure)| (id, figure.clone()))
            .collect();
        let result = engine
            .execute(&format!(
                "disp('retained output');{command};answer=helper();function y=helper();y=9;end"
            ))
            .unwrap();
        assert_eq!(engine.workspace().len(), 1);
        assert_eq!(engine.variables["answer"].data(), &[9.]);
        assert!(!result.clear_console);
        assert!(result.output.iter().any(|s| s.contains("retained output")));
        for (id, before) in figures {
            let (_, after) = engine.figures().find(|(n, _)| *n == id).unwrap();
            assert!(before.same_snapshot(after));
        }
    }
}

#[test]
fn clear_item_type_variable_precedence_uses_the_initial_workspace() {
    for name in [
        "all",
        "variables",
        "functions",
        "classes",
        "import",
        "java",
        "mex",
    ] {
        let mut engine = seed();
        engine
            .execute(&format!("{name}=logical([0 1]);clear {name} {name};"))
            .unwrap();
        assert!(engine.array(name).is_none());
        assert_eq!(engine.variables["keep"].data(), &[77.]);
    }
    for command in [
        "clear all",
        "clear('all')",
        "clear functions",
        "clear classes",
        "clear import",
        "clear java",
        "clear mex",
        "clear A variables",
    ] {
        let mut engine = seed();
        let before = engine.workspace();
        assert!(engine.execute(command).is_err(), "Accepted {command}");
        assert_unchanged(&engine, &before);
    }
    // Reference behavior: clear global removes global variables only; local variables stay.
    let mut engine = seed();
    let before = engine.workspace();
    engine.execute("clear global").unwrap();
    assert_unchanged(&engine, &before);
}

#[test]
fn clear_rejections_are_atomic_and_nonliteral_arguments_are_not_evaluated() {
    for command in [
        "clear('A','')",
        "clear('A','bad-name')",
        "clear('A','A*')",
        "clear('A','a/b')",
        "clear('A','while')",
        "clear A -regexp ^B",
        "clear('A',1)",
        "clear('A',logical(1))",
        "clear('A',side_effect());function y=side_effect();disp('SHOULD_NOT_RUN');plot([1 2]);y=1;end",
    ] {
        let mut engine = seed();
        let before = engine.workspace();
        assert!(engine.execute(command).is_err(), "Accepted {command}");
        assert_unchanged(&engine, &before);
        assert!(engine.figure.is_none());
        assert!(engine.failed_output.output.is_empty());
    }
    let mut engine = seed();
    let before = engine.workspace();
    assert!(
        engine
            .execute(&format!("clear('A','{}')", "x".repeat(64)))
            .is_err()
    );
    assert_unchanged(&engine, &before);
}

#[test]
fn clear_variable_and_callable_shadowing_remain_ordinary() {
    let mut engine = seed();
    engine
        .execute("clear=[5 8];clear;indexed=clear(2);plus=clear + 1;")
        .unwrap();
    assert_eq!(engine.variables["indexed"].data(), &[8.]);
    assert_eq!(engine.variables["plus"].data(), &[6., 9.]);
    assert!(engine.array("A").is_some());
    engine.execute("clear '").unwrap();
    assert_eq!(
        (
            engine.variables["ans"].rows(),
            engine.variables["ans"].cols()
        ),
        (2, 1)
    );
    assert!(engine.execute("clear('A')").is_err()); // variable indexing, never built-in deletion
    assert!(engine.array("A").is_some());
    let mut engine = seed();
    engine
        .execute("clear=logical([0 1]);clear;indexed=clear(2);")
        .unwrap();
    // A bare variable name echoes under its own name and leaves `ans` unset (MATLAB).
    assert!(engine.array("ans").is_none());
    assert_eq!(engine.logical_variables["indexed"].data(), &[1]);
    assert!(engine.array("A").is_some());
    assert!(engine.execute("clear('A')").is_err());
    assert!(engine.array("A").is_some());
    let mut engine = seed();
    engine
        .execute("answer=clear(5);function y=clear(x);y=x+2;end")
        .unwrap();
    assert_eq!(engine.variables["answer"].data(), &[7.]);
    assert!(engine.array("A").is_some());
    engine.execute("clear;function y=clear();y=12;end").unwrap();
    assert_eq!(engine.variables["ans"].data(), &[12.]);
    assert!(engine.array("A").is_some());
}

#[test]
fn clear_is_function_local_and_missing_outputs_restore_the_caller() {
    let mut engine = seed();
    let pointer = engine.variables["A"].data().as_ptr();
    engine
        .execute("answer=helper(A);function y=helper(x);mask=logical(1);clear x mask;y=9;end")
        .unwrap();
    assert_eq!(engine.variables["answer"].data(), &[9.]);
    assert_eq!(engine.variables["A"].data().as_ptr(), pointer);
    assert!(engine.logical_variables.contains_key("mask"));
    engine
        .execute("answer=helper(A);function y=helper(x);clear;y=4;end")
        .unwrap();
    assert_eq!(engine.variables["answer"].data(), &[4.]);
    let before = engine.workspace();
    assert!(
        engine
            .execute("answer=helper(A);function y=helper(x);y=x;clear y;end")
            .is_err()
    );
    assert_unchanged(&engine, &before);
}

#[test]
fn clear_unloads_retained_functions_but_same_named_variables_take_precedence() {
    let mut engine = seed();
    engine
        .execute("helper=3;clear helper;answer=helper();function y=helper();y=4;end")
        .unwrap();
    assert_eq!(engine.variables["answer"].data(), &[4.]);
    // Reference behavior: clear name removes a variable and unloads a function of that name.
    engine
        .execute("clear A helper;function y=helper();y=4;end")
        .unwrap();
    assert!(engine.array("A").is_none());
    engine.execute("A=[1 2 3];").unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("helper.m"), "function y=helper();y=6;end").unwrap();
    std::fs::write(dir.path().join("unloaded.m"), [0xff, 0xfe]).unwrap();
    engine.set_function_directory(dir.path()).unwrap();
    engine.execute("clear unloaded;").unwrap(); // does not open named files
    // Reference behavior: clearing a loaded folder function unloads it with the variables.
    engine.execute("answer=helper();clear A helper;").unwrap();
    assert!(engine.array("A").is_none());
    assert_eq!(engine.variables["answer"].data(), &[6.]);
    engine.execute("A=[1 2 3];").unwrap();
    std::fs::write(dir.path().join("clear.m"), "function y=clear(x);y=x+10;end").unwrap();
    engine.execute("answer=clear(2);").unwrap();
    assert_eq!(engine.variables["answer"].data(), &[12.]);
    assert!(engine.array("A").is_some());
}

#[test]
fn clear_output_and_nested_forms_do_not_delete_or_evaluate_arguments() {
    for source in [
        "result=clear('A');",
        "keep(1)=clear('A');",
        "[x,y]=clear('A');",
        "sin(clear('A'));",
        "result=clear(side_effect());function y=side_effect();disp('SHOULD_NOT_RUN');y=1;end",
        "[x,y]=clear(side_effect());function y=side_effect();disp('SHOULD_NOT_RUN');y=1;end",
    ] {
        let mut engine = seed();
        let before = engine.workspace();
        assert!(engine.execute(source).is_err(), "Accepted {source}");
        assert_unchanged(&engine, &before);
        assert!(engine.failed_output.output.is_empty());
    }
}

#[test]
fn clear_255_names_fit_parser_bounds_and_overlimit_does_not_delete() {
    let mut engine = Engine::default();
    let names: Vec<_> = (0..255).map(|i| format!("v{i}")).collect();
    for name in &names {
        engine.variables.insert(name.clone(), Matrix::scalar(1.));
    }
    let before = engine.workspace();
    assert!(
        engine
            .execute(&format!("clear {} extra", names.join(" ")))
            .is_err()
    );
    assert_unchanged(&engine, &before);
    engine
        .execute(&format!("clear {}", names.join(" ")))
        .unwrap();
    assert!(engine.workspace().is_empty());
    for name in &names {
        engine.variables.insert(name.clone(), Matrix::scalar(1.));
    }
    let quoted: Vec<_> = names.iter().map(|n| format!("'{n}'")).collect();
    engine
        .execute(&format!("clear({})", quoted.join(",")))
        .unwrap();
    assert!(engine.workspace().is_empty());
}
