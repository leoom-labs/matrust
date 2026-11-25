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
        "clear functions",
        "clear java",
        "clear mex",
        "clear('java')",
        "clear('mex')",
    ] {
        let mut engine = seed();
        let before = engine.workspace();
        let result = engine
            .execute(command)
            .unwrap_or_else(|error| panic!("{command}: {error}"));
        assert!(result.output.is_empty(), "{command}");
        assert!(!result.clear_console, "{command}");
        assert_unchanged(&engine, &before);
    }
    for command in [
        "clear all",
        "clear('all')",
        "clear -all",
        "clear('-all')",
        "clear classes",
        "clear('classes')",
    ] {
        let mut engine = seed();
        let result = engine
            .execute(command)
            .unwrap_or_else(|error| panic!("{command}: {error}"));
        assert!(engine.workspace().is_empty(), "{command}");
        assert!(
            result.output.is_empty(),
            "{command}: {}",
            result.output.join("\n")
        );
        assert!(!result.clear_console, "{command}");
    }
    for command in ["clear A variables", "clear all extra", "clear A functions"] {
        let mut engine = seed();
        let before = engine.workspace();
        assert!(engine.execute(command).is_err(), "Accepted {command}");
        assert_unchanged(&engine, &before);
    }
    // clear import empties the import list; variables stay.
    for command in ["clear import", "clear('import')"] {
        let mut engine = seed();
        let before = engine.workspace();
        engine.execute(command).unwrap();
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
        "clear('A','B*','bad-name')",
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
        "result=clear('all');",
        "result=clear('functions');",
        "result=clear('classes');",
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

fn counter() -> &'static str {
    "function y=counter(), persistent n; if isempty(n), n=0; end; n=n+1; y=n; end"
}

#[test]
fn clear_all_and_clear_functions_reset_persistent_variables() {
    let counter = counter();
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "y1=counter(); y2=counter(); clear all; y3=counter(); {counter}"
        ))
        .unwrap();
    assert!(engine.array("y1").is_none() && engine.array("y2").is_none());
    assert_eq!(engine.variables["y3"].data(), &[1.]);
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "keep=5; y1=counter(); y2=counter(); clear functions; y3=counter(); {counter}"
        ))
        .unwrap();
    assert_eq!(engine.variables["keep"].data(), &[5.]);
    assert_eq!(engine.variables["y1"].data(), &[1.]);
    assert_eq!(engine.variables["y2"].data(), &[2.]);
    assert_eq!(engine.variables["y3"].data(), &[1.]);
    // clear classes removes the variables and leaves the persistent count.
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "y1=counter(); y2=counter(); clear classes; y3=counter(); {counter}"
        ))
        .unwrap();
    assert!(engine.array("y1").is_none() && engine.array("y2").is_none());
    assert_eq!(engine.variables["y3"].data(), &[3.]);
}

// MathWorks clear page: "If a function is locked or currently running, it is
// not cleared from memory." Its persistent variables stay, also for its next
// call; those of functions that are not running are reset.
#[test]
fn clear_all_and_clear_functions_keep_persistents_of_running_functions() {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        (
            "cnt.m",
            "function r = cnt()\npersistent n\nif isempty(n), n = 0; end\nn = n + 1;\nclear functions\nr = n;\nend\n",
        ),
        (
            "outer.m",
            "function r = outer()\npersistent n\nif isempty(n), n = 0; end\nn = n + 1;\ninner();\nr = n;\nend\n",
        ),
        ("inner.m", "function inner()\nclear all\nend\n"),
        (
            "cb.m",
            "function r = cb()\npersistent calls\nif isempty(calls), calls = 0; end\ncalls = calls + 1;\nevalin('base', 'clear all');\nr = calls;\nend\n",
        ),
        // Cleared before this call reached its persistent statement.
        (
            "late.m",
            "function r = late()\nclear functions\npersistent n\nif isempty(n), n = 0; end\nn = n + 1;\nr = n;\nend\n",
        ),
        (
            "counter.m",
            "function y = counter()\npersistent n\nif isempty(n), n = 0; end\nn = n + 1;\ny = n;\nend\n",
        ),
    ];
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let mut engine = Engine::default();
    engine.set_current_folder(dir.path()).unwrap();
    let mut value = |code: &str, name: &str| {
        engine
            .execute(code)
            .unwrap_or_else(|error| panic!("{code}: {error}"));
        engine.variables[name].data().to_vec()
    };
    assert_eq!(value("a = cnt();", "a"), [1.]);
    assert_eq!(value("a = cnt();", "a"), [2.]);
    assert_eq!(value("y = counter(); y = counter();", "y"), [2.]);
    // outer runs while inner clears: outer keeps counting, counter restarts.
    assert_eq!(value("b = outer();", "b"), [1.]);
    assert_eq!(value("v = [outer() counter()];", "v"), [2., 1.]);
    assert_eq!(value("c = cb();", "c"), [1.]);
    assert_eq!(value("c = cb();", "c"), [2.]);
    assert_eq!(value("d = late(); d = late();", "d"), [2.]);
    // From the top level no function runs: every count restarts.
    assert_eq!(
        value("clear functions; v = [cnt() outer() cb()];", "v"),
        [1., 1., 1.]
    );
}

#[test]
fn clear_global_and_clear_all_empty_the_global_store() {
    let mut engine = Engine::default();
    engine
        .execute(
            "global a b; a=3; b=4; keep=9; clear global a; global a; na=numel(a); nb=numel(b);",
        )
        .unwrap();
    assert_eq!(engine.variables["na"].data(), &[0.]);
    assert_eq!(engine.variables["nb"].data(), &[1.]);
    assert_eq!(engine.variables["b"].data(), &[4.]);
    assert_eq!(engine.variables["keep"].data(), &[9.]);
    engine
        .execute("clear global; global a b; na=numel(a); nb=numel(b);")
        .unwrap();
    assert_eq!(engine.variables["na"].data(), &[0.]);
    assert_eq!(engine.variables["nb"].data(), &[0.]);
    assert_eq!(engine.variables["keep"].data(), &[9.]);
    engine
        .execute("global a; a=3; local=4; clear all; global a; na=numel(a);")
        .unwrap();
    assert_eq!(engine.variables["na"].data(), &[0.]);
    assert!(engine.array("local").is_none() && engine.array("keep").is_none());
    // clear functions does not touch globals or workspace variables.
    engine
        .execute("global a; a=4; keep=1; clear functions; na=numel(a);")
        .unwrap();
    assert_eq!(engine.variables["na"].data(), &[1.]);
    assert_eq!(engine.variables["a"].data(), &[4.]);
    assert_eq!(engine.variables["keep"].data(), &[1.]);
}

#[test]
fn clear_all_keeps_figures_path_folder_and_builtins() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("extra")).unwrap();
    let mut engine = Engine::default();
    engine.set_current_folder(dir.path()).unwrap();
    engine.execute("plot([1 2 3]); addpath('extra');").unwrap();
    assert_eq!(engine.figures().count(), 1);
    let folder = engine.current_folder().unwrap().to_path_buf();
    engine.execute("clear all; p = path; b = abs(-2);").unwrap();
    assert_eq!(engine.figures().count(), 1);
    assert_eq!(engine.current_folder(), Some(folder.as_path()));
    assert_eq!(engine.variables["b"].data(), &[2.]);
    let path = match engine.workspace().get("p").unwrap() {
        matrust::Array::Character(text) => String::from_utf16_lossy(text.units()),
        other => panic!("path: {}", other.class()),
    };
    assert!(path.contains("extra"), "{path}");
}

#[test]
fn clear_all_and_clear_functions_leave_the_running_program() {
    let mut engine = Engine::default();
    engine
        .execute("clear all; y=helper(); function y=helper(); y=4; end")
        .unwrap();
    assert_eq!(engine.variables["y"].data(), &[4.]);
    let mut engine = Engine::default();
    engine
        .execute("keep=3; clear functions; y=helper(); function y=helper(); y=4; end")
        .unwrap();
    assert_eq!(engine.variables["keep"].data(), &[3.]);
    assert_eq!(engine.variables["y"].data(), &[4.]);
}

#[test]
fn clear_all_drops_the_import_list_and_reloads_package_functions() {
    let dir = tempfile::tempdir().unwrap();
    let package = dir.path().join("+a");
    std::fs::create_dir(&package).unwrap();
    std::fs::write(
        package.join("fn.m"),
        "function y = fn(x)\ny = x + 4;\nend\n",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_current_folder(dir.path()).unwrap();
    engine
        .execute("import a.fn; y1 = fn(1); clear all; n = numel(import);")
        .unwrap();
    assert!(engine.array("y1").is_none());
    assert_eq!(engine.variables["n"].data(), &[0.]);
    assert!(engine.execute("y2 = fn(1);").is_err());
    engine.execute("y3 = a.fn(1);").unwrap();
    assert_eq!(engine.variables["y3"].data(), &[5.]);
}

#[test]
fn clear_classes_warns_and_keeps_a_class_that_still_has_an_instance() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Base.m"),
        "classdef Base\n    properties\n        b = 3\n    end\n    methods\n        function obj = Base()\n        end\n        function y = mark(obj)\n            y = 3;\n        end\n    end\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Live.m"),
        "classdef Live < Base\n    properties\n        v = 1\n    end\n    methods\n        function obj = Live()\n            obj = obj@Base();\n        end\n    end\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Other.m"),
        "classdef Other\n    properties\n        v = 1\n    end\n    methods\n        function obj = Other()\n        end\n    end\nend\n",
    )
    .unwrap();
    let other = dir.path().join("Other.m");
    let base = dir.path().join("Base.m");
    let mut engine = Engine::default();
    engine.set_current_folder(dir.path()).unwrap();
    // The rewrite is inside this evaluation. The file cache is not checked by
    // mtime, so Other() stays at v = 1 until clear classes drops that class.
    let result = engine
        .execute(&format!(
            "global g mem; g = Live(); o1 = Other(); before = o1.v; \
             fid = fopen('{}', 'w'); \
             fprintf(fid, 'classdef Other\\n    properties\\n        v = 7\\n    end\\n    methods\\n        function obj = Other()\\n        end\\n    end\\nend\\n'); \
             fclose(fid); o2 = Other(); mid = o2.v; mem = [before mid]; \
             clear classes; global g mem; gv = g.v; o3 = Other(); ov = o3.v; \
             fid = fopen('{}', 'w'); \
             fprintf(fid, 'classdef Base\\n    properties\\n        b = 9\\n    end\\n    methods\\n        function obj = Base()\\n        end\\n        function y = mark(obj)\\n            y = 9;\\n        end\\n    end\\nend\\n'); \
             fclose(fid); nb = Base(); nbb = nb.b; mk = mark(nb);",
            other.display().to_string().replace('\'', "''"),
            base.display().to_string().replace('\'', "''"),
        ))
        .unwrap_or_else(|error| panic!("{error}"));
    let warning = result.output.join("\n");
    assert!(
        warning.contains(
            "Objects of 'Live' class exist. Cannot clear this class or any of its superclasses."
        ),
        "{warning}"
    );
    assert!(!warning.contains("Objects of 'Base'"), "{warning}");
    assert!(!warning.contains("Objects of 'Other'"), "{warning}");
    assert_eq!(engine.variables["mem"].data(), &[1., 1.]);
    assert_eq!(engine.variables["gv"].data(), &[1.]);
    assert_eq!(engine.variables["ov"].data(), &[7.]);
    assert_eq!(engine.variables["nbb"].data(), &[3.]);
    assert_eq!(engine.variables["mk"].data(), &[3.]);
}

#[test]
fn clear_all_close_all_and_clc_run_the_course_script() {
    let mut engine = Engine::default();
    engine.execute("plot([1 2 3]);").unwrap();
    assert_eq!(engine.figures().count(), 1);
    let result = engine
        .execute("clear all; close all; clc; x = 1; disp(x)")
        .unwrap();
    assert!(result.clear_console);
    assert!(
        result.output.iter().any(|line| line.contains('1')),
        "{:?}",
        result.output
    );
    assert_eq!(engine.variables["x"].data(), &[1.]);
    assert_eq!(engine.figures().count(), 0);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"))
        .args(["-batch", "clear all; close all; clc; x = 1; disp(x)"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains('1'), "{stdout}");
}
