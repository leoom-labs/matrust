use matrust::{Engine, Matrix, syntax};
use std::sync::atomic::Ordering;

fn scalar(e: &Engine, name: &str) -> f64 {
    e.variables[name].data()[0]
}

#[test]
fn local_functions_have_private_scopes_shared_inputs_and_typed_returns() {
    let mut e = Engine::default();
    e.execute("A=[1 2 3];B=A;[C,mask]=change(A);D=helper(4);\nfunction [y,m]=change(x)\nx(1)=9;y=x;m=x>2;temp=42;\nend\nfunction y=helper(x)\ny=x*2;\nend").unwrap();
    assert_eq!(e.variables["A"].data(), &[1., 2., 3.]);
    assert_eq!(
        e.variables["B"].data().as_ptr(),
        e.variables["A"].data().as_ptr()
    );
    assert_eq!(e.variables["C"].data(), &[9., 2., 3.]);
    assert_eq!(e.logical_variables["mask"].data(), &[1, 0, 1]);
    assert_eq!(scalar(&e, "D"), 8.);
    for n in ["x", "y", "m", "temp"] {
        assert!(e.array(n).is_none());
    }
    let error = e.execute("helper(1);").unwrap_err();
    assert!(error.contains("helper"), "{error}");
    let before = e.variables["A"].data().as_ptr();
    e.execute("C=identity(A);function y=identity(x);y=x;end")
        .unwrap();
    assert_eq!(e.variables["C"].data().as_ptr(), before);
    assert!(
        e.execute("saved=7;R=leak();function y=leak();y=saved;end")
            .is_err()
    );
    assert_eq!(scalar(&e, "saved"), 7.);
    assert!(e.array("R").is_none());
}

#[test]
fn optional_inputs_outputs_and_return_preserve_call_context() {
    let mut e = Engine::default();
    e.execute("A=optional(4);[B,N,M]=optional(4,3);[~,Q]=partial(7);\nfunction [y,ni,no]=optional(x,factor)\nni=nargin;no=nargout();if nargin<2;factor=2;end;y=x*factor;return;y=999;end\nfunction [unused,y]=partial(x);y=x;end").unwrap();
    assert_eq!(scalar(&e, "A"), 8.);
    assert_eq!(scalar(&e, "B"), 12.);
    assert_eq!(scalar(&e, "N"), 2.);
    assert_eq!(scalar(&e, "M"), 3.);
    assert_eq!(scalar(&e, "Q"), 7.);
    e.execute("[~]=partial();function y=partial;end").unwrap();
    e.execute("bare();function y=bare();if nargout==0;disp(17);return;end;y=1;end")
        .unwrap();
    assert!(!e.variables.contains_key("ans"));
    e.execute("bare();function y=bare;y=23;end").unwrap();
    assert_eq!(scalar(&e, "ans"), 23.);
    e.execute("R=noparens;function y=noparens;y=5;end").unwrap();
    assert_eq!(scalar(&e, "R"), 5.);
    for source in [
        "A=f(1,2);function y=f(x);y=x;end",
        "[A,B]=f();function y=f;y=1;end",
        "A=f();function y=f(x);y=x;end",
        "A=f();function f;disp(1);end",
        "[A,B]=f();function [x,y]=f;x=1;end",
    ] {
        let old = e.variables["A"].clone();
        assert!(e.execute(source).is_err(), "{source}");
        assert_eq!(e.variables["A"].data().as_ptr(), old.data().as_ptr());
    }
}

#[test]
fn recursive_functions_unwind_errors_and_bound_native_recursion() {
    let mut e = Engine::default();
    e.execute(
        "R=factorial(10);function y=factorial(n);if n<=1;y=1;else;y=n*factorial(n-1);end;end",
    )
    .unwrap();
    assert_eq!(scalar(&e, "R"), 3628800.);
    let err = e
        .execute("keep=9;R=forever(1);function y=forever(x);y=forever(x);end")
        .unwrap_err();
    assert!(err.contains("depth exceeds 32"), "{err}");
    assert_eq!(scalar(&e, "R"), 3628800.);
    assert_eq!(scalar(&e, "keep"), 9.);
    assert!(!e.variables.contains_key("x"));
    // Each recursive call retains many pending expression frames.
    let expression = format!("{}deep(x){}", "1+(".repeat(45), ")".repeat(45));
    let error = e
        .execute(&format!("R=deep(1);function y=deep(x);y={expression};end"))
        .unwrap_err();
    assert!(error.contains("128 levels"), "{error}");
    let nested = format!(
        "R=deep();function y=deep;{}y=deep();{}end",
        "if true;".repeat(62),
        "end;".repeat(62)
    );
    let error = e.execute(&nested).unwrap_err();
    assert!(
        error.contains("block/function execution exceeds 128"),
        "{error}"
    );
    e.execute("R=double_it(2);function y=double_it(x);y=x*2;end")
        .unwrap();
    assert_eq!(scalar(&e, "R"), 4.);
}

#[test]
fn malformed_function_programs_reject_before_mutating_the_workspace() {
    for source in [
        "x=1;function y=f(x);y=x;",
        "x=1;function f;end;function f;end",
        "x=1;function y=f(a,a);y=a;end",
        "x=1;function [y,y]=f;y=1;end",
        "x=1;function y=f(a,);y=a;end",
        "x=1;if true;function f;end;end",
        "function f;function g;end;end",
        "function f;end;x=1;",
        "function f;end;function g",
        // Reference behavior: varargin is accepted; it must be the last input.
        "x=1;function y=f(varargin,a);y=1;end",
    ] {
        let mut e = Engine::default();
        assert!(e.execute(source).is_err(), "{source}");
        assert!(e.workspace().is_empty(), "{source}");
    }
    let many = (0..129)
        .map(|n| format!("function f{n};end;"))
        .collect::<String>();
    assert!(syntax::parse(&many).unwrap_err().contains("128 functions"));
}

#[test]
fn function_files_resolve_local_helpers_reload_between_runs_and_remain_private() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("calculate.m"),
        "function y=calculate(x)\ny=helper(x);\nfunction y=helper(x)\ny=x+3;",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_function_directory(folder.path()).unwrap();
    e.execute("R=calculate(4);").unwrap();
    assert_eq!(scalar(&e, "R"), 7.);
    assert!(e.execute("R=helper(4);").is_err());
    std::fs::write(
        folder.path().join("calculate.m"),
        "function y=calculate(x);y=x*3;end",
    )
    .unwrap();
    e.execute("R=calculate(4);").unwrap();
    assert_eq!(scalar(&e, "R"), 12.);
    std::fs::write(folder.path().join("sum.m"), "function y=sum(x);y=19;end").unwrap();
    e.execute("R=sum([1 2]);").unwrap();
    assert_eq!(scalar(&e, "R"), 19.);
    e.execute("R=sum([1 2]);function y=sum(x);y=23;end")
        .unwrap();
    assert_eq!(scalar(&e, "R"), 23.);
    e.execute("sum=[8 9];R=sum(2);").unwrap();
    assert_eq!(scalar(&e, "R"), 9.);
    std::fs::write(folder.path().join("bad.m"), "function y=wrong;y=1;end").unwrap();
    assert!(
        e.execute("bad();")
            .unwrap_err()
            .contains("must be named 'bad'")
    );
    std::fs::write(folder.path().join("bad.m"), "x=1;").unwrap();
    assert!(
        e.execute("bad(1);")
            .unwrap_err()
            .contains("Attempt to execute SCRIPT bad as a function:\n")
    );
}

#[test]
fn function_failures_report_inner_local_locations_and_external_file_context() {
    let mut e = Engine::default();
    let source = "% α\nkept=7;R=outer(1);\nfunction y=outer(x)\ny=inner(x);\nend\nfunction y=inner(x)\n  missing;\nend";
    let error = e.execute(source).unwrap_err();
    assert!(
        error.contains("In inner") && error.contains("In outer"),
        "{error}"
    );
    assert_eq!(e.diagnostic.as_ref().unwrap().line, 7);
    assert_eq!(e.diagnostic.as_ref().unwrap().column, 3);
    assert_eq!(scalar(&e, "kept"), 7.);
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("broken.m"),
        "function y=broken(x)\ny=missing;\nend",
    )
    .unwrap();
    e.set_function_directory(folder.path()).unwrap();
    let error = e.execute("R=broken(1);").unwrap_err();
    assert!(
        error.contains("broken.m") && error.contains("Line 2"),
        "{error}"
    );
    assert_eq!(e.diagnostic.as_ref().unwrap().line, 1);
    let file = e.file_diagnostic.as_ref().unwrap();
    assert_eq!(file.diagnostic.line, 2);
    assert!(file.path.ends_with("broken.m"));
    assert!(file.source.contains("y=missing;"));
    e.execute("R=1;").unwrap();
    assert!(e.file_diagnostic.is_none());
}

#[test]
fn external_diagnostics_keep_innermost_source_and_parse_locations() {
    let folder = tempfile::tempdir().unwrap();
    let outer = folder.path().join("outer.m");
    let inner = folder.path().join("inner.m");
    std::fs::write(outer, "function y=outer(x)\ny=inner(x);\nend").unwrap();
    let source = "function y=inner(x)\n% Unicode: αβ\n  y=missing;\nend";
    std::fs::write(&inner, source).unwrap();
    let mut e = Engine::default();
    e.set_function_directory(folder.path()).unwrap();
    assert!(e.execute("R=outer(1);").is_err());
    let file = e.file_diagnostic.as_ref().unwrap();
    assert!(file.path.ends_with("inner.m"));
    assert_eq!(&*file.source, source);
    assert_eq!((file.diagnostic.line, file.diagnostic.column), (3, 3));
    let invalid = "function y=inner(x)\ny=(;\nend";
    std::fs::write(&inner, invalid).unwrap();
    assert!(e.execute("R=outer(1);").is_err());
    let file = e.file_diagnostic.as_ref().unwrap();
    assert!(file.path.ends_with("inner.m"));
    assert_eq!(&*file.source, invalid);
    assert_eq!(file.diagnostic.line, 2);
    assert!(e.execute("R=missing;").is_err());
    assert!(e.file_diagnostic.is_none());
}

#[test]
fn function_calls_preserve_global_execution_and_retained_memory_limits() {
    let mut e = Engine::default();
    let error = e
        .execute("while true;spin();end;function spin;for k=1:1000;end;end")
        .unwrap_err();
    assert!(error.contains("1000000"), "{error}");
    let error = e
        .execute("while true;show();end;function show;disp(1);end")
        .unwrap_err();
    assert!(error.contains("2000"), "{error}");
    assert_eq!(e.failed_output.output.len(), 2000);
    let large = Matrix::filled(1, 8 * 1024 * 1024, 1.).unwrap();
    e.variables.insert("A".into(), large);
    let error = e
        .execute("R=chain(A);function y=chain(x);y=chain(x);end")
        .unwrap_err();
    assert!(error.contains("Retained function caller"), "{error}");
    assert_eq!(e.variables["A"].len(), 8 * 1024 * 1024);
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(e.execute("R=7;").unwrap_err().contains("cancelled"));
    e.cancellation().store(false, Ordering::Relaxed);
    e.execute("clear;R=1;").unwrap();
    assert_eq!(scalar(&e, "R"), 1.);
}

#[test]
fn function_example_matches_full_filtering_and_keeps_helper_variables_private() {
    let mut e = Engine::default();
    e.execute(include_str!("../examples/user_functions.m"))
        .unwrap();
    assert!(scalar(&e, "error") < 1e-13);
    assert_eq!(e.figure.as_ref().unwrap().series.len(), 2);
    assert_eq!(e.variables["final_state"].len(), 48);
    for name in ["chunk", "first", "last", "part", "x", "y", "state"] {
        assert!(e.array(name).is_none(), "{name}");
    }
}

#[test]
fn cancellation_inside_function_restores_caller_and_allows_the_next_run() {
    let mut e = Engine::default();
    e.execute("keep=11;").unwrap();
    let cancellation = e.cancellation();
    let stop = cancellation.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(10));
        stop.store(true, Ordering::Relaxed);
    });
    let error = e
        .execute("blocked();function blocked;while true;x=sin(ones(1,65536));end;end")
        .unwrap_err();
    thread.join().unwrap();
    assert!(error.contains("cancelled"), "{error}");
    assert_eq!(scalar(&e, "keep"), 11.);
    assert!(!e.variables.contains_key("x"));
    cancellation.store(false, Ordering::Relaxed);
    e.execute("R=ok();function y=ok;y=17;end").unwrap();
    assert_eq!(scalar(&e, "R"), 17.);
}

#[test]
fn function_file_size_count_and_total_source_limits_are_enforced() {
    let folder = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_function_directory(folder.path()).unwrap();
    e.execute("R=17;").unwrap();
    std::fs::write(folder.path().join("bad.m"), vec![b' '; 1_000_001]).unwrap();
    assert!(e.execute("R=bad();").unwrap_err().contains("1 MB"));
    assert_eq!(scalar(&e, "R"), 17.);
    std::fs::write(folder.path().join("bad.m"), [0xff, 0xfe]).unwrap();
    assert!(e.execute("R=bad();").is_err());
    assert_eq!(scalar(&e, "R"), 17.);
    let mut calls = String::new();
    for i in 0..65 {
        std::fs::write(
            folder.path().join(format!("f{i}.m")),
            format!("function y=f{i};y={i};end"),
        )
        .unwrap();
        calls.push_str(&format!("R=f{i}();"));
    }
    assert!(e.execute(&calls).unwrap_err().contains("64 function files"));
    assert_eq!(scalar(&e, "R"), 63.);
    calls.clear();
    for i in 0..9 {
        let mut source = format!("function y=large{i};y={i};end\n%");
        source.push_str(&" ".repeat(1_000_000 - source.len()));
        std::fs::write(folder.path().join(format!("large{i}.m")), source).unwrap();
        calls.push_str(&format!("R=large{i}();"));
    }
    assert!(e.execute(&calls).unwrap_err().contains("8 MiB"));
    assert_eq!(scalar(&e, "R"), 7.);
    assert!(
        e.set_function_directory(&folder.path().join("missing"))
            .is_err()
    );
    e.execute("R=f1();").unwrap();
    assert_eq!(scalar(&e, "R"), 1.);
}

#[test]
fn shared_matlab_probe_cases_match_independent_expected_values() {
    let mut e = Engine::default();
    e.execute(include_str!(
        "../tests/fixtures/scripts/user_function_cases.m"
    ))
    .unwrap();
    assert_eq!(
        e.variables["ans"].data(),
        &[
            1., 2., 3., 9., 2., 3., 1., 0., 1., 10., 15., 2., 3., 8., 7., 0., 8., 2., 720., -4., 3.
        ]
    );
}

#[test]
fn cli_resolves_sibling_function_files_and_runs_zero_input_function_files() {
    let folder = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("scale.m"),
        "function y=scale(x);y=x*7;end",
    )
    .unwrap();
    let script = folder.path().join("driver.m");
    std::fs::write(&script, "R=scale(6)\n").unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"))
        .arg(&script)
        .arg("--threads")
        .arg("1")
        .current_dir(other.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("R =\n\n    42\n"));
    std::fs::write(&script, "function y=driver;y=8;end").unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"))
        .arg(&script)
        .arg("--threads")
        .arg("1")
        .current_dir(other.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("ans =\n\n     8\n"));
}

#[test]
fn omitted_inputs_cannot_resolve_as_constants_context_builtins_or_function_files() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("missing_input.m"),
        "function y=missing_input;y=99;end",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_function_directory(folder.path()).unwrap();
    for input in ["pi", "true", "nargin", "nargout", "missing_input"] {
        let source = format!("R=helper();function y=helper({input});y={input};end");
        let error = e.execute(&source).unwrap_err();
        assert!(error.contains("was not supplied"), "{input}: {error}");
        assert!(!e.variables.contains_key("R"));
    }
    e.execute("R=helper();function y=helper(pi);if nargin<1;pi=7;end;y=pi;end")
        .unwrap();
    assert_eq!(scalar(&e, "R"), 7.);
}
