use matrust::{Engine, Matrix, files};
use std::path::Path;

#[test]
fn indexed_assignment_matches_matlab_including_errors_and_shapes() {
    let cases: Vec<String> = serde_json::from_str(include_str!(
        "fixtures/indexing/indexed_assignment_cases.json"
    ))
    .unwrap();
    let reference = files::load_workspace(Path::new(
        "tests/fixtures/indexing/indexed-assignment-oracle.mat",
    ))
    .unwrap();
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                for (i, source) in cases.iter().enumerate() {
                    let mut engine = Engine::default();
                    let result = engine.execute(source);
                    let expected_ok = reference[&format!("ok{:03}", i + 1)].data()[0] == 1.;
                    assert_eq!(
                        result.is_ok(),
                        expected_ok,
                        "case {}: {source}: {result:?}",
                        i + 1
                    );
                    let expected_exists = reference[&format!("exists{:03}", i + 1)].data()[0] == 1.;
                    assert_eq!(
                        engine.variables.contains_key("A"),
                        expected_exists,
                        "case {}: {source}",
                        i + 1
                    );
                    if expected_exists {
                        let actual = &engine.variables["A"];
                        let expected = &reference[&format!("r{:03}", i + 1)];
                        assert_eq!(
                            (actual.rows(), actual.cols()),
                            (expected.rows(), expected.cols()),
                            "case {}: {source}",
                            i + 1
                        );
                        assert_eq!(actual.data(), expected.data(), "case {}: {source}", i + 1);
                    }
                }
            });
    }
}

#[test]
fn writes_preserve_aliases_and_use_bounded_unique_commits() {
    let mut e = Engine::default();
    e.execute("A=reshape(1:120,10,12);").unwrap();
    let before = e.variables["A"].data().as_ptr();
    e.execute("A(3,2)=777;").unwrap();
    assert_eq!(e.variables["A"].data().as_ptr(), before);
    assert_eq!(e.variables["A"].get(2, 1), 777.);
    e.execute("B=A; C=reshape(A,12,10); A([3 3],2)=[8 9];")
        .unwrap();
    assert_ne!(e.variables["A"].data().as_ptr(), before);
    assert_eq!(e.variables["A"].get(2, 1), 9.);
    assert_eq!(e.variables["B"].get(2, 1), 777.);
    assert_eq!(e.variables["C"].data()[12], 777.);
    e.execute("D=zeros(10,12); D(:)=B;").unwrap();
    assert_eq!(
        e.variables["D"].data().as_ptr(),
        e.variables["B"].data().as_ptr()
    );
    let output = e.execute("D(1)=55").unwrap();
    assert!(output.output[0].starts_with("\nD =\n"));
    assert_eq!(e.variables["B"].get(0, 0), 1.);
    e.execute("D([2 3 2])=D([1 2 3]);").unwrap();
    assert_eq!(&e.variables["D"].data()[..3], &[55., 3., 2.]);
}

#[test]
fn indexed_writes_reject_invalid_targets_and_preserve_workspace() {
    let mut e = Engine::default();
    for script in [
        "A()=1;",
        "A(1,2,3)=4;",
        "A(1)+2=3;",
        "[A(0),B]=size(1);",
        "A(1)=B(1)=2;",
    ] {
        assert!(e.execute(script).is_err(), "{script}");
        assert!(e.variables.is_empty());
    }
    e.execute("A=reshape(1:6,2,3);").unwrap();
    let original = e.variables["A"].clone();
    for script in [
        "A(16777217,1)=3;",
        "A(4097,4096)=3;",
        "A(:,4)=[1 2 3];",
        "A(0)=4;",
        "A(1)='text';",
        "missing(1)=[];",
    ] {
        assert!(e.execute(script).is_err(), "{script}");
        assert_eq!(e.variables["A"].data().as_ptr(), original.data().as_ptr());
        assert_eq!(e.variables.len(), 1);
    }
    e.cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(e.execute("A(:)=9;").unwrap_err().contains("cancelled"));
    assert_eq!(e.variables["A"].data(), original.data());
}

#[test]
fn growth_preflights_logical_workspace_without_replacing_any_array() {
    let mut e = Engine::default();
    let large = Matrix::filled(1, 16 * 1024 * 1024, 3.).unwrap();
    e.variables.insert("L".into(), large.clone());
    e.variables.insert("M".into(), large);
    assert!(e.execute("A(1)=1;").unwrap_err().contains("Workspace"));
    assert!(!e.variables.contains_key("A"));
    assert_eq!(
        e.variables["L"].data().as_ptr(),
        e.variables["M"].data().as_ptr()
    );
    assert!(e.execute("L(end+1)=1;").is_err());
    assert_eq!(e.variables["L"].len(), 16 * 1024 * 1024);
}

#[test]
fn signal_edit_example_preserves_original_and_updates_plot_samples() {
    let mut e = Engine::default();
    e.execute(include_str!("fixtures/indexing/indexed_assignment.m"))
        .unwrap();
    assert_eq!(
        (e.variables["data"].rows(), e.variables["data"].cols()),
        (900, 3)
    );
    assert_eq!(
        (
            e.variables["original"].rows(),
            e.variables["original"].cols()
        ),
        (1000, 2)
    );
    assert_eq!(e.variables["t"].len(), 900);
    for i in 100..120 {
        assert_eq!(e.variables["data"].get(i, 1), 0.);
    }
    assert!(
        e.variables["original"].data()[1200..1220]
            .iter()
            .any(|&x| x != 0.)
    );
    assert_eq!(e.figure.as_ref().unwrap().series.len(), 2);
    assert_eq!(e.figure.as_ref().unwrap().series[0].points.len(), 900);
}

#[test]
fn large_writes_growth_and_reversal_agree_across_worker_counts() {
    let mut outputs = Vec::new();
    for threads in [1, 8] {
        let result = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let mut e = Engine::default();
                // Odd row count makes copy chunks cross column boundaries after growth.
                let rows = 65537;
                e.execute("U=zeros(65537,8);").unwrap();
                let pointer = e.variables["U"].data().as_ptr();
                e.execute("U(:,3)=7; U(:,[4 4])=[ones(65537,1)*8 ones(65537,1)*9];")
                    .unwrap();
                assert_eq!(e.variables["U"].data().as_ptr(), pointer);
                for r in 0..65537 {
                    assert_eq!(e.variables["U"].get(r, 2), 7.);
                    assert_eq!(e.variables["U"].get(r, 3), 9.);
                }

                e.variables.insert(
                    "A".into(),
                    Matrix::from_column_vec(rows, 8, (0..rows * 8).map(|i| i as f64).collect())
                        .unwrap(),
                );
                e.execute("saved=A; A(:,[3 3])=[ones(65537,1)*7 ones(65537,1)*9]; A(end+2,10)=11;")
                    .unwrap();
                assert_eq!(
                    (e.variables["A"].rows(), e.variables["A"].cols()),
                    (rows + 2, 10)
                );
                for c in 0..10 {
                    for r in 0..rows + 2 {
                        let expected = if r == rows + 1 && c == 9 {
                            11.
                        } else if r >= rows || c >= 8 {
                            0.
                        } else if c == 2 {
                            9.
                        } else {
                            (c * rows + r) as f64
                        };
                        assert_eq!(e.variables["A"].get(r, c), expected);
                    }
                }
                e.execute("B=A; B(end:-1:1,end:-1:1)=A;").unwrap();
                assert!(
                    e.variables["B"]
                        .data()
                        .iter()
                        .zip(e.variables["A"].data().iter().rev())
                        .all(|(a, b)| a == b)
                );
                e.execute("A(1:3:end,[9 9])=4; A(:,[2 2 5])=[]; B(:)=3;")
                    .unwrap();
                assert!(e.variables["B"].data().iter().all(|&x| x == 3.));
                assert_eq!(
                    (e.variables["saved"].rows(), e.variables["saved"].cols()),
                    (rows, 8)
                );
                e.variables.remove("A").unwrap()
            });
        outputs.push(result);
    }
    assert_eq!(outputs[0].data(), outputs[1].data());
}
