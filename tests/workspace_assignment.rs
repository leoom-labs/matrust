use matrust::{Array, Engine, LogicalMatrix, Matrix};
use std::sync::atomic::Ordering;

#[test]
fn single_assignments_account_exact_typed_bytes_and_preserve_rejected_targets() {
    let mut e = Engine::default();
    let payload = LogicalMatrix::filled(1, 16 * 1024 * 1024, false, &e.cancellation()).unwrap();
    e.merge_workspace(
        (0..16)
            .map(|i| (format!("L{i}"), Array::Logical(payload.clone())))
            .collect(),
    )
    .unwrap();
    assert_eq!(e.workspace_bytes(), 256 * 1024 * 1024);
    let original = e.logical_variables["L0"].data().as_ptr();
    assert!(e.execute("X=1;").unwrap_err().contains("256 MiB"));
    assert!(!e.variables.contains_key("X"));
    assert_eq!(e.logical_variables["L0"].data().as_ptr(), original);

    e.execute("L0=false;").unwrap();
    assert!(
        e.execute("X=ones(1,2097152);")
            .unwrap_err()
            .contains("256 MiB")
    );
    assert!(!e.variables.contains_key("X"));
    e.execute("X=zeros(1,2097151);").unwrap();
    let pointer = e.variables["X"].data().as_ptr();
    e.execute("X=X;L0=1;").unwrap();
    assert_eq!(e.variables["X"].data().as_ptr(), pointer);
    assert!(!e.logical_variables.contains_key("L0"));
    assert_eq!(e.workspace_bytes(), 256 * 1024 * 1024);
    assert!(e.execute("7").unwrap_err().contains("256 MiB"));
    assert!(!e.variables.contains_key("ans"));
    assert!(e.failed_output.output.is_empty());
    e.execute("L0=true;").unwrap();
    assert!(e.execute("ans=7;").unwrap_err().contains("256 MiB"));
    let output = e.execute("L0=[];7").unwrap();
    assert_eq!(e.workspace_bytes(), 256 * 1024 * 1024);
    assert_eq!(e.variables["ans"].data(), &[7.]);
    assert!(output.output[0].starts_with("\nans ="));
}

#[test]
fn direct_workspace_changes_are_counted_and_type_replacement_preserves_aliases() {
    let mut e = Engine::default();
    e.execute("A=[1 2];B=A;").unwrap();
    let snapshot = e.variables["A"].clone();
    e.execute("A=true;A=A;A=[3 4];").unwrap();
    assert_eq!(snapshot.data(), &[1., 2.]);
    assert_eq!(e.variables["B"].data().as_ptr(), snapshot.data().as_ptr());
    assert_eq!(e.variables["A"].data(), &[3., 4.]);
    assert!(!e.logical_variables.contains_key("A"));

    // Library callers can change either public map between execute calls.
    let empty = Matrix::filled(0, 0, 0.).unwrap();
    for i in 0..9998 {
        e.variables.insert(format!("v{i}"), empty.clone());
    }
    assert!(e.execute("extra=1;").unwrap_err().contains("10000"));
    e.execute("A=false;").unwrap();
    assert_eq!(e.variables.len() + e.logical_variables.len(), 10000);
    e.variables.remove("v0");
    e.execute("extra=1;").unwrap();
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(e.execute("A=42;").unwrap_err().contains("cancelled"));
    assert_eq!(e.logical_variables["A"].data(), &[0]);
}

#[test]
fn single_output_failures_and_output_budget_keep_prior_assignment_semantics() {
    let mut e = Engine::default();
    e.execute("A=7;").unwrap();
    assert!(
        e.execute("A=\"text\";")
            .unwrap_err()
            .contains("String array storage")
    );
    assert!(
        e.execute("A=disp(3);")
            .unwrap_err()
            .contains("does not return")
    );
    assert_eq!(e.variables["A"].data(), &[7.]);
    assert_eq!(e.failed_output.output.len(), 1);
    let error = e.execute("for k=1:2001;A=k\nend").unwrap_err();
    assert!(error.contains("output"));
    assert_eq!(e.failed_output.output.len(), 2000);
    assert_eq!(e.variables["A"].data(), &[2001.]);
}
