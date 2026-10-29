use matrust::Engine;

fn select(e: &mut Engine, document: &str, code: &str) -> matrust::Result<matrust::Execution> {
    let start = document.find(code).unwrap();
    e.execute_selection(document, start..start + code.len())
}

#[test]
fn selections_use_unsaved_local_functions_without_running_other_statements() {
    let mut e = Engine::default();
    let source = "% αβ\nnever=missing;\nR=twice(7);\nalso_never=missing;\nfunction y=twice(x)\ny=helper(x)+helper(x);\nend\nfunction y=helper(x)\ny=x+1;\nend";
    select(&mut e, source, "R=twice(7);").unwrap();
    assert_eq!(e.variables["R"].data(), &[16.]);
    assert_eq!(e.variables.len(), 1);
    let edited = source.replace("y=x+1", "y=x+2");
    select(&mut e, &edited, "R=twice(7);").unwrap();
    assert_eq!(e.variables["R"].data(), &[18.]);
    assert!(e.execute("twice(7);").is_err());
}

#[test]
fn selection_scope_precedence_and_function_definitions_do_not_invoke_primary() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sin.m"), "function y=sin(x);y=999;end").unwrap();
    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();
    let source = "skip=missing;\nR=sin(3);\nfunction y=sin(x)\ny=x*2;\nend";
    select(&mut e, source, "R=sin(3);").unwrap();
    assert_eq!(e.variables["R"].data(), &[6.]);
    let functions = "function y=primary(x)\ny=missing;\nend\nfunction y=helper(x)\ny=x;\nend";
    e.execute_selection(functions, 0..functions.len()).unwrap();
    assert!(!e.variables.contains_key("ans"));
    e.execute("x=4;").unwrap();
    select(&mut e, functions, "y=x;").unwrap();
    assert_eq!(e.variables["y"].data(), &[4.]);
}

#[test]
fn selection_errors_use_full_document_coordinates_and_keep_completed_work() {
    let mut e = Engine::default();
    let source = "% αβ\nnever=missing;\ngood=7;bad=missing;\nfunction y=helper(x)\n  y=missing;\nend\nR=helper(1);";
    assert!(select(&mut e, source, "good=7;bad=missing;").is_err());
    let d = e.diagnostic.as_ref().unwrap();
    assert_eq!((d.line, d.column), (3, 8));
    assert_eq!(e.variables["good"].data(), &[7.]);
    assert!(select(&mut e, source, "R=helper(1);").is_err());
    let d = e.diagnostic.as_ref().unwrap();
    assert_eq!((d.line, d.column), (5, 3));
    assert!(!e.variables.contains_key("never"));
    let source = "% αβ\nR=(1+2);";
    assert!(select(&mut e, source, "R=(1+").is_err());
    assert_eq!(e.diagnostic.as_ref().unwrap().line, 2);
    e.execute("done=1;").unwrap();
    assert!(e.diagnostic.is_none());
}

#[test]
fn selections_validate_ranges_source_and_context_before_mutating_workspace() {
    let mut e = Engine::default();
    let source = "% αβ\nR=1;";
    for range in [3..4, 0..usize::MAX, std::ops::Range { start: 5, end: 2 }] {
        assert!(e.execute_selection(source, range).is_err());
        assert!(e.variables.is_empty());
    }
    e.execute_selection(source, source.len()..source.len())
        .unwrap();
    assert!(e.variables.is_empty());
    let source = "R=1;\nfunction y=f(x);y=x;end\nfunction y=f(x);y=x;end";
    assert!(
        select(&mut e, source, "R=1;")
            .unwrap_err()
            .contains("Duplicate")
    );
    let source = "R=1;\nbad=(;";
    assert!(select(&mut e, source, "R=1;").is_err());
    assert!(e.variables.is_empty());
    assert!(
        e.execute_selection(&" ".repeat(1_000_001), 0..0)
            .unwrap_err()
            .contains("1 MB")
    );
}

#[test]
fn selection_function_calls_preserve_arrays_and_external_diagnostics() {
    let mut e = Engine::default();
    e.execute("A=[1 2 3];").unwrap();
    let pointer = e.variables["A"].data().as_ptr();
    let source = "skip=missing;\n[B,M]=keep(A);\nfunction [y,m]=keep(x)\ny=x;m=x>1;\nend";
    select(&mut e, source, "[B,M]=keep(A);").unwrap();
    assert_eq!(e.variables["B"].data().as_ptr(), pointer);
    assert_eq!(e.logical_variables["M"].data(), &[0, 1, 1]);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("broken.m"),
        "function y=broken(x)\ny=missing;\nend",
    )
    .unwrap();
    e.set_function_directory(dir.path()).unwrap();
    let source = "% α\nnever=missing;\nR=broken(1);";
    assert!(select(&mut e, source, "R=broken(1);").is_err());
    assert_eq!(e.diagnostic.as_ref().unwrap().line, 3);
    assert_eq!(e.file_diagnostic.as_ref().unwrap().diagnostic.line, 2);
    assert!(
        e.file_diagnostic
            .as_ref()
            .unwrap()
            .path
            .ends_with("broken.m")
    );
}
