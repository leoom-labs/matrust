use matrust::{Engine, syntax};
use std::sync::atomic::Ordering;

fn scalar(e: &Engine, name: &str) -> f64 {
    e.array(name)
        .unwrap()
        .scalar(0, 0)
        .unwrap()
        .into_real()
        .unwrap()
}

#[test]
fn nested_conditionals_choose_one_branch_and_skip_unneeded_expressions() {
    let mut e = Engine::default();
    e.execute("x=0; if false; missing; elseif [1 2;3 4]; if 2>1; x=7; else; x=8; end; elseif missing; x=9; else; x=10; end").unwrap();
    assert_eq!(scalar(&e, "x"), 7.);
    for (condition, expected) in [
        ("[]", 0.),
        ("[1 0]", 0.),
        ("[1 -1]", 1.),
        ("false & missing", 0.),
        ("true | missing", 1.),
        ("~(false & missing)", 1.),
        ("[true false] | [false true]", 1.),
        ("false && missing", 0.),
    ] {
        e.execute(&format!("x=0;if {condition}, x=1;end")).unwrap();
        assert_eq!(scalar(&e, "x"), expected, "{condition}");
    }
    e.execute("if false, x=1; else if true, x=2; end; end")
        .unwrap();
    assert_eq!(scalar(&e, "x"), 2.);
}

#[test]
fn for_ranges_nested_loops_and_matrix_columns_follow_fixed_inputs() {
    let mut e = Engine::default();
    e.execute("A=zeros(4,6);for c=1:6;for r=1:4;if r==c;A(r,c)=2;elseif abs(r-c)==1;A(r,c)=-1;end;end;end").unwrap();
    for c in 0..6 {
        for r in 0..4 {
            assert_eq!(
                e.variables["A"].get(r, c),
                if r == c {
                    2.
                } else if r.abs_diff(c) == 1 {
                    -1.
                } else {
                    0.
                }
            );
        }
    }
    e.execute("sumv=0;for v=5:-2:1;sumv=sumv+v;end").unwrap();
    assert_eq!(scalar(&e, "sumv"), 9.);
    assert_eq!(scalar(&e, "v"), 1.);
    e.execute("A=[1 2 3;4 5 6];snapshot=A;count=0;total=0;for v=A;count=count+1;total=total+sum(v);A(:)=0;end").unwrap();
    assert_eq!(scalar(&e, "count"), 3.);
    assert_eq!(scalar(&e, "total"), 21.);
    assert_eq!(e.variables["v"].data(), &[3., 6.]);
    assert_eq!(e.variables["snapshot"].data(), &[1., 4., 2., 5., 3., 6.]);
    e.execute("count=0;for v=[1;2;3];count=count+1;end")
        .unwrap();
    assert_eq!(scalar(&e, "count"), 1.);
    assert_eq!((e.variables["v"].rows(), e.variables["v"].cols()), (3, 1));
}

#[test]
fn loop_values_are_typed_and_zero_iterations_replace_the_index_with_empty() {
    let mut e = Engine::default();
    e.execute("s=0;for flag=[true false true];s=s+flag;end")
        .unwrap();
    assert_eq!(scalar(&e, "s"), 2.);
    assert!(e.array("flag").unwrap().is_logical());
    e.execute("k=9;for k=3:1;missing;end").unwrap();
    assert!(e.variables["k"].is_empty());
    e.execute("for flag=false(1,0);missing;end").unwrap();
    assert!(e.array("flag").unwrap().is_logical());
    assert!(e.array("flag").unwrap().is_empty());
    e.execute("n=0;for k=1:3;n=n+1;k=99;end").unwrap();
    assert_eq!(scalar(&e, "n"), 3.);
    assert_eq!(scalar(&e, "k"), 99.);
    e.execute("k=99;snapshot=k;for k=1:3;if k==2;saved=k;end;end")
        .unwrap();
    assert_eq!(scalar(&e, "snapshot"), 99.);
    assert_eq!(scalar(&e, "saved"), 2.);
    assert_eq!(scalar(&e, "k"), 3.);
}

#[test]
fn empty_variables_and_names_cannot_bypass_workspace_metadata_limits() {
    let mut e = Engine::default();
    let empty = matrust::Matrix::filled(0, 0, 0.).unwrap();
    let updates = (0..10000)
        .map(|i| (format!("v{i}"), matrust::Array::Numeric(empty.clone())))
        .collect();
    e.merge_workspace(updates).unwrap();
    assert!(e.execute("extra=[];").unwrap_err().contains("10000"));
    assert_eq!(e.variables.len(), 10000);
    e.execute("v0=7;").unwrap();
    assert_eq!(scalar(&e, "v0"), 7.);
    e.execute("clear;").unwrap();
    assert!(
        e.execute(&format!("{}=1;", "a".repeat(64)))
            .unwrap_err()
            .contains("63")
    );
    assert!(e.execute("_invalid=1;").is_err());
    assert!(e.variables.is_empty());
    let updates =
        std::collections::BTreeMap::from([("while".into(), matrust::Array::Numeric(empty))]);
    assert!(e.merge_workspace(updates).is_err());
    // Class-block section names are not reserved MATLAB keywords globally.
    e.execute(
        "methods=1;properties=2;events=3;enumeration=4;R=methods+properties+events+enumeration;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "R"), 10.);
}

#[test]
fn break_continue_and_return_unwind_the_correct_scope() {
    let mut e = Engine::default();
    e.execute("s=0;for k=1:10;if k==2;continue;end;if k==5;break;end;s=s+k;end;after=1;")
        .unwrap();
    assert_eq!(scalar(&e, "s"), 8.);
    assert_eq!(scalar(&e, "k"), 5.);
    e.execute("n=0;for a=1:3;for b=1:10;n=n+1;break;end;end")
        .unwrap();
    assert_eq!(scalar(&e, "n"), 3.);
    e.execute("n=0;while n<5;n=n+1;if n<3;continue;end;break;end")
        .unwrap();
    assert_eq!(scalar(&e, "n"), 3.);
    let out = e
        .execute("n=0;for k=1:9;while true;n=n+1;disp(n);return;end;end;unreachable=1;")
        .unwrap();
    assert_eq!(scalar(&e, "n"), 1.);
    assert!(!e.variables.contains_key("unreachable"));
    assert_eq!(out.output.len(), 1);
    e.execute("after_return=7;").unwrap();
    assert_eq!(scalar(&e, "after_return"), 7.);
}

#[test]
fn malformed_blocks_and_unsupported_keywords_fail_before_any_execution() {
    for source in [
        "if true;x=1;",
        "else;x=1;",
        "end",
        "elseif true;x=1;end",
        "if true;else;elseif true;end",
        "for k 1:3;end",
        "for [a,b]=1:3;end",
        "while;end",
        "break",
        "if true;continue;end",
        "return 7",
        "if true;function f();end;end",
        "switch 1;x=1;end",
        "switch 1;case 1;x=1;",
        "parfor k=1:3;end",
        "if true x=1;end",
        "for if=1:3;end",
        "[for,x]=size(1);",
        "x=for;",
    ] {
        let mut e = Engine::default();
        assert!(
            e.execute(&format!("before=7;{source}")).is_err(),
            "{source}"
        );
        assert!(e.variables.is_empty(), "{source}");
    }
    let deep = format!("{}{}", "if true;".repeat(65), "end;".repeat(65));
    assert!(syntax::parse(&deep).unwrap_err().contains("64"));
    let allowed = format!("{}x=1;{}", "if true;".repeat(64), "end;".repeat(64));
    assert!(Engine::default().execute(&allowed).is_ok());
}

#[test]
fn nested_errors_retain_precise_locations_and_completed_results() {
    let mut e = Engine::default();
    let source = "% α\nfor k=1:3\n  x=k;\n  if k==2\n    missing(1);\n  end\nend\nafter=1;";
    assert!(e.execute(source).is_err());
    let d = e.diagnostic.as_ref().unwrap();
    assert_eq!((d.line, d.column), (5, 5));
    assert_eq!(scalar(&e, "x"), 2.);
    assert!(!e.variables.contains_key("after"));
    assert!(
        e.execute("if false\n x=1;\nelseif missing\n x=2;\nend")
            .is_err()
    );
    assert_eq!(e.diagnostic.as_ref().unwrap().line, 3);
}

#[test]
fn empty_infinite_loops_and_nested_iteration_are_bounded() {
    let mut e = Engine::default();
    let err = e.execute("kept=7;while true;end").unwrap_err();
    assert!(err.contains("1000000"), "{err}");
    assert_eq!(scalar(&e, "kept"), 7.);
    assert_eq!(e.diagnostic.as_ref().unwrap().column, 8);
    e.execute("after=1;").unwrap();
    let err = e
        .execute("for a=1:10000;for b=1:10000;end;end")
        .unwrap_err();
    assert!(err.contains("1000000"));
}

#[test]
fn loop_output_is_bounded_even_across_clc_and_partial_state_is_retained() {
    let mut e = Engine::default();
    let err = e.execute("for k=1:3000;disp(k);end").unwrap_err();
    assert!(err.contains("2000"));
    assert_eq!(e.failed_output.output.len(), 2000);
    assert_eq!(scalar(&e, "k"), 2001.);
    let err = e.execute("while true;disp('x');clc;end").unwrap_err();
    assert!(err.contains("output"));
    assert!(e.failed_output.output.is_empty());
    let large = "x".repeat(800_000);
    let err = e
        .execute(&format!("for k=1:4;disp('{large}');end"))
        .unwrap_err();
    assert!(err.contains("2 MiB"));
    assert_eq!(e.failed_output.output.len(), 2);
}

#[test]
fn active_loop_inputs_have_a_separate_memory_budget_and_release_on_failure() {
    let mut e = Engine::default();
    e.execute("A=true(1,16777216);").unwrap();
    let source = format!(
        "{}{}",
        (0..17).map(|i| format!("for k{i}=A;")).collect::<String>(),
        "end;".repeat(17)
    );
    let err = e.execute(&source).unwrap_err();
    assert!(err.contains("Active for-loop inputs"), "{err}");
    e.execute("for k=[1 2];x=k;end").unwrap();
    assert_eq!(scalar(&e, "x"), 2.);
}

#[test]
fn running_loops_observe_cooperative_cancellation() {
    let mut e = Engine::default();
    e.execute("kept=7;").unwrap();
    let cancel = e.cancellation();
    let stop = cancel.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(10));
        stop.store(true, Ordering::Relaxed);
    });
    let err = e
        .execute("while true;A=sin(ones(1,65536));end")
        .unwrap_err();
    thread.join().unwrap();
    assert!(err.contains("cancelled"), "{err}");
    assert_eq!(scalar(&e, "kept"), 7.);
    cancel.store(false, Ordering::Relaxed);
    e.execute("x=1;").unwrap();
}

#[test]
fn control_flow_filter_example_matches_continuous_filtering_and_plots_all_orders() {
    let mut e = Engine::default();
    e.execute(include_str!("../examples/control_flow_filtering.m"))
        .unwrap();
    assert_eq!(scalar(&e, "filters_run"), 3.);
    assert_eq!(scalar(&e, "chunks_run"), 24.);
    assert_eq!(e.figure.as_ref().unwrap().series.len(), 4);
    e.execute("whole=filter(b,1,signal);difference=norm(whole-clean);")
        .unwrap();
    assert!(scalar(&e, "difference") < 1e-13);
}
