use matrust::{Array, Engine, Matrix, files};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

fn check(script: &str, shape: (usize, usize), logical: bool, expected: &[f64]) {
    let mut e = Engine::default();
    e.execute(script)
        .unwrap_or_else(|err| panic!("{script}: {err}"));
    let a = e.array("R").unwrap();
    assert_eq!((a.rows(), a.cols()), shape, "{script}");
    assert_eq!(a.is_logical(), logical, "{script}");
    let n = a.to_numeric(&AtomicBool::new(false)).unwrap();
    assert_eq!(n.data().len(), expected.len());
    for (&x, &y) in n.data().iter().zip(expected) {
        assert!(x == y || (x.is_nan() && y.is_nan()), "{script}: {x} != {y}");
    }
}

#[test]
fn script_comparison_precedence_short_circuit_and_types() {
    for s in [
        "R=1<2==1;",
        "R=1|0&0;",
        "R=1||0&&0;",
        "R=true||missing;",
        "R=~2^0==false;",
    ] {
        check(s, (1, 1), true, &[1.]);
    }
    check("R=false&&missing;", (1, 1), true, &[0.]);
    check("R=2^~0;", (1, 1), false, &[2.]);
    check("R=1:3>1;", (1, 3), true, &[0., 1., 1.]);
    check(
        "R=[true false;false true]';",
        (2, 2),
        true,
        &[1., 0., 0., 1.],
    );
    check("R=[logical([]),true];", (1, 1), true, &[1.]);
    check("R=[true;logical([])];", (1, 1), true, &[1.]);
    check("R=[logical([])];", (0, 0), true, &[]);
    check("R=[true 2];", (1, 2), false, &[1., 2.]);
    check(
        "R=reshape(logical([1 0 1 0]),2,2);",
        (2, 2),
        true,
        &[1., 0., 1., 0.],
    );
    check("R=true+true;", (1, 1), false, &[2.]);
    check("R=-true;", (1, 1), false, &[-1.]);
    check("R=double(false(2,3));", (2, 3), false, &[0.; 6]);
    check("R=true([2 3]);", (2, 3), true, &[1.; 6]);
    for s in [
        "R=true&&missing;",
        "R=false||missing;",
        "R=[true false]&&true;",
        "R=[]&&true;",
        "R=logical(NaN);",
        "R=[1 2]>[1;2];",
        "R=true(1,2,3);",
        "R=1!=2;",
    ] {
        assert!(Engine::default().execute(s).is_err(), "{s}");
    }
}

#[test]
fn masks_select_values_in_column_major_order_and_preserve_vector_orientation() {
    check(
        "A=[10 20 30];R=A([false true false]);",
        (1, 1),
        false,
        &[20.],
    );
    check(
        "A=[10;20;30];R=A([true false true]);",
        (2, 1),
        false,
        &[10., 30.],
    );
    check(
        "A=[10 20 30];R=A([true;false;true]);",
        (1, 2),
        false,
        &[10., 30.],
    );
    check("A=reshape(1:6,2,3);R=A(A>3);", (3, 1), false, &[4., 5., 6.]);
    check(
        "A=reshape(1:6,2,3);R=A([true false true]);",
        (2, 1),
        false,
        &[1., 3.],
    );
    check(
        "A=[10 20 30 40];R=A(logical([1 0;0 1]));",
        (2, 1),
        false,
        &[10., 40.],
    );
    check(
        "A=[10 20 30];R=A([false false false false]);",
        (1, 0),
        false,
        &[],
    );
    check(
        "A=reshape(1:6,2,3);R=A([true false],[false true true]);",
        (1, 2),
        false,
        &[3., 5.],
    );
    check(
        "A=reshape(1:6,2,3);R=A([false false false],:);",
        (0, 3),
        false,
        &[],
    );
    check(
        "A=logical([1 0 1]);R=A([true false true]);",
        (1, 2),
        true,
        &[1., 1.],
    );
    for s in [
        "A=[1 2 3];R=A([false false false true]);",
        "A=ones(2);R=A([false false true],:);",
    ] {
        assert!(Engine::default().execute(s).is_err());
    }
}

#[test]
fn masks_assign_grow_delete_and_retain_destination_type() {
    check("A=[10 20 30];A(A>10)=0;R=A;", (1, 3), false, &[10., 0., 0.]);
    check(
        "A=[10 20 30];A([false false false true])=7;R=A;",
        (1, 4),
        false,
        &[10., 20., 30., 7.],
    );
    check(
        "A=reshape(1:6,2,3);A([false false true],:)=7;R=A;",
        (3, 3),
        false,
        &[1., 2., 7., 3., 4., 7., 5., 6., 7.],
    );
    check(
        "A=[10 20 30];A([false true false])=[];R=A;",
        (1, 2),
        false,
        &[10., 30.],
    );
    check(
        "A=reshape(1:6,2,3);A(:,[false true false])=[];R=A;",
        (2, 2),
        false,
        &[1., 2., 5., 6.],
    );
    check(
        "A=logical([1 0 1]);A(2)=3;R=A;",
        (1, 3),
        true,
        &[1., 1., 1.],
    );
    check(
        "A=logical([1 0 1]);A(5)=true;R=A;",
        (1, 5),
        true,
        &[1., 0., 1., 0., 1.],
    );
    check("A=logical([1 0 1]);A(2)=[];R=A;", (1, 2), true, &[1., 1.]);
    check("A(3)=true;R=A;", (1, 3), true, &[0., 0., 1.]);
    check("A=zeros(1,3);A(2)=true;R=A;", (1, 3), false, &[0., 1., 0.]);
    check(
        "A=logical([1 0;0 1]);A([1 3],[2 3])=logical([0 1;1 0]);R=A;",
        (3, 3),
        true,
        &[1., 0., 0., 0., 1., 1., 1., 0., 0.],
    );
    check(
        "A=logical([1 0;0 1]);A(:,true(1,2))=[];R=A;",
        (2, 0),
        true,
        &[],
    );
    check(
        "A=logical([1 0 1]);A(A)=false;R=A;",
        (1, 3),
        true,
        &[0., 0., 0.],
    );
    check("A=true(2);A(:)=[];R=A;", (0, 0), true, &[]);
}

#[test]
fn logical_aliases_type_replacements_and_failed_writes_are_transactional() {
    let mut e = Engine::default();
    e.execute("A=logical([1 0 1]);B=A;C=A(:);D=reshape(A,3,1);")
        .unwrap();
    let ptr = e.logical_variables["A"].data().as_ptr();
    for n in ["B", "C", "D"] {
        assert_eq!(ptr, e.logical_variables[n].data().as_ptr());
    }
    for s in [
        "A(2)=NaN;",
        "A(0)=true;",
        "A([1 2])=true(3);",
        "A([false false false true])=[];",
    ] {
        assert!(e.execute(s).is_err(), "{s}");
        assert_eq!(e.logical_variables["A"].data(), &[1, 0, 1]);
    }
    e.execute("A(2)=true;").unwrap();
    assert_eq!(e.logical_variables["B"].data(), &[1, 0, 1]);
    assert_ne!(e.logical_variables["A"].data().as_ptr(), ptr);
    let unique = e.logical_variables["A"].data().as_ptr();
    e.execute("A(1)=false;").unwrap();
    assert_eq!(e.logical_variables["A"].data().as_ptr(), unique);
    e.execute("A=[3 4];B=B+1;").unwrap();
    assert!(!e.logical_variables.contains_key("A"));
    assert_eq!(e.variables["A"].data(), &[3., 4.]);
    e.execute("A=A>3;").unwrap();
    assert!(!e.variables.contains_key("A"));
    assert_eq!(e.logical_variables["A"].data(), &[0, 1]);
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(e.execute("A(:)=true;").is_err());
    assert_eq!(e.logical_variables["A"].data(), &[0, 1]);
    e.cancellation().store(false, Ordering::Relaxed);
    e.execute("clear;").unwrap();
    assert!(e.workspace().is_empty());
}

#[test]
fn logical_shapes_reductions_and_information_functions() {
    check("[r,c]=size(true(2,3));R=[r c];", (1, 2), false, &[2., 3.]);
    check(
        "R=[islogical(true),isnumeric(true),isnumeric(1),islogical(1)];",
        (1, 4),
        true,
        &[1., 0., 1., 0.],
    );
    check(
        "R=[numel(false(2,3)),length(false(2,3)),size(false(2,3),2)];",
        (1, 3),
        false,
        &[6., 3., 3.],
    );
    check("R=isnan([0 NaN Inf]);", (1, 3), true, &[0., 1., 0.]);
    check("R=isfinite([NaN Inf 1]);", (1, 3), true, &[0., 0., 1.]);
    check("R=any([0 NaN;0 0]);", (1, 2), true, &[0., 0.]);
    check("R=all([1 NaN;1 1]);", (1, 2), true, &[1., 1.]);
    check("R=any(zeros(0,3));", (1, 3), true, &[0.; 3]);
    check("R=all(zeros(0,3));", (1, 3), true, &[1.; 3]);
    check("R=all([]);", (1, 1), true, &[1.]);
    check("R=nnz([0 NaN Inf -1]);", (1, 1), false, &[3.]);
    check("R=find([0 2;3 0]);", (2, 1), false, &[2., 3.]);
    check("R=find([false true false true]);", (1, 2), false, &[2., 4.]);
    check("R=sum(true(2,3));", (1, 3), false, &[2.; 3]);
    check("R=mean(false(2,3));", (1, 3), false, &[0.; 3]);
}

#[test]
fn typed_workspace_budget_includes_masks_and_multi_output_replacements() {
    let large = Matrix::filled(16 * 1024 * 1024, 1, 0.).unwrap();
    let mut e = Engine::default();
    e.variables.insert("A".into(), large.clone());
    e.variables.insert("B".into(), large);
    assert!(e.execute("mask=true;").unwrap_err().contains("Workspace"));
    assert!(!e.logical_variables.contains_key("mask"));
    e.execute("A=true(1024);mask=true;").unwrap();
    assert_eq!(e.workspace_bytes(), 128 * 1024 * 1024 + 1024 * 1024 + 1);
    e.execute("[mask,A]=size(B);").unwrap();
    assert!(e.logical_variables.is_empty());
    assert_eq!(e.variables["mask"].data(), &[16777216.]);
}

#[test]
fn typed_mat_roundtrip_keeps_masks_and_legacy_api_remains_numeric() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typed.mat");
    let flag = AtomicBool::new(false);
    let mut e = Engine::default();
    e.execute("A=[1 NaN;Inf -0];mask=A>1;empty=false(0,3);tiny=true;")
        .unwrap();
    for compressed in [false, true] {
        if compressed {
            files::save_array_workspace(&path, &e.workspace(), &flag).unwrap();
        } else {
            files::save_array_workspace_uncompressed(&path, &e.workspace(), &flag).unwrap();
        }
        let catalog = files::inspect_workspace(&path, &flag).unwrap();
        assert!(catalog.variables().all(|v| v.compressed == compressed));
        let names = catalog.variables().map(|v| v.name.clone()).collect();
        let vars = files::load_selected_arrays(&catalog, &names, &flag).unwrap();
        for (n, original) in e.workspace() {
            let a = &vars[&n];
            assert_eq!(a.is_logical(), original.is_logical());
            assert_eq!((a.rows(), a.cols()), (original.rows(), original.cols()));
            for r in 0..a.rows() {
                for c in 0..a.cols() {
                    assert_eq!(
                        a.scalar(r, c).unwrap().into_real().unwrap().to_bits(),
                        original
                            .scalar(r, c)
                            .unwrap()
                            .into_real()
                            .unwrap()
                            .to_bits()
                    );
                }
            }
        }
        let legacy = files::load_selected_workspace(&catalog, &names, &flag).unwrap();
        assert_eq!(legacy["mask"].data(), &[0., 1., 0., 0.]);
        assert_eq!(
            catalog
                .variables()
                .find(|v| v.name == "mask")
                .unwrap()
                .storage_bytes(),
            Some(4)
        );
        let prior = std::fs::read(&path).unwrap();
        flag.store(true, Ordering::Relaxed);
        assert!(files::save_array_workspace(&path, &vars, &flag).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), prior);
        assert!(files::load_selected_arrays(&catalog, &names, &flag).is_err());
        flag.store(false, Ordering::Relaxed);
    }
}

#[test]
fn merged_numeric_import_replaces_a_logical_name_once() {
    let mut e = Engine::default();
    e.execute("A=true;B=3;").unwrap();
    e.merge_workspace(BTreeMap::from([(
        "A".into(),
        Array::Numeric(Matrix::scalar(4.)),
    )]))
    .unwrap();
    assert_eq!(e.workspace().len(), 2);
    assert!(e.logical_variables.is_empty());
}

#[test]
fn independent_scipy_logical_fixtures_decode_without_double_expansion() {
    let flag = AtomicBool::new(false);
    for file in [
        "tests/fixtures/mat/scipy-v6.mat",
        "tests/fixtures/mat/scipy-v7.mat",
    ] {
        let catalog = files::inspect_workspace(std::path::Path::new(file), &flag).unwrap();
        let names = catalog.variables().map(|v| v.name.clone()).collect();
        let data = files::load_selected_arrays(&catalog, &names, &flag).unwrap();
        assert_eq!(data["mask"].bytes(), 6);
        assert!(data["mask"].is_logical());
        assert_eq!(data["mask"].scalar(0, 0).unwrap().into_real().unwrap(), 1.);
        assert_eq!(data["mask"].scalar(1, 0).unwrap().into_real().unwrap(), 0.);
        assert_eq!((data["empty"].rows(), data["empty"].cols()), (0, 3));
        let dense = &data["dense"];
        assert_eq!(
            (dense.rows(), dense.cols(), dense.bytes()),
            (257, 129, 257 * 129)
        );
        for c in 0..129 {
            for r in 0..257 {
                assert_eq!(
                    dense.scalar(r, c).unwrap().into_real().unwrap(),
                    if (r * 17 + c * 13) % 11 < 5 { 1. } else { 0. }
                );
            }
        }
        assert_eq!(
            data["numeric"]
                .scalar(1, 0)
                .unwrap()
                .into_real()
                .unwrap()
                .to_bits(),
            (-0.0f64).to_bits()
        );
        assert!(
            data["numeric"]
                .scalar(0, 1)
                .unwrap()
                .into_real()
                .unwrap()
                .is_nan()
        );
    }
}

#[test]
fn logical_selection_example_plots_original_and_selected_samples() {
    let mut e = Engine::default();
    e.execute(include_str!("../examples/logical_selection.m"))
        .unwrap();
    let mask = &e.logical_variables["selected"];
    let count = mask.data().iter().filter(|v| **v != 0).count();
    assert_eq!(e.variables["selected_count"].data(), &[count as f64]);
    assert!(count > 0 && count < 2000);
    let fig = e.figure.unwrap();
    assert_eq!(fig.series.len(), 3);
    assert_eq!(fig.series[2].points.len(), count);
    for (i, &v) in mask.data().iter().enumerate() {
        let signal = e.variables["signal"].data()[i];
        let clean = e.variables["clean"].data()[i];
        assert_eq!(clean.is_nan(), v != 0);
        if v == 0 {
            assert_eq!(clean, signal);
        }
    }
}

#[test]
fn large_masks_cover_parallel_chunks_tails_contiguous_sharing_and_transactions() {
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let n = 4 * 1024 * 1024 + 3;
                let flag = AtomicBool::new(false);
                let source =
                    Matrix::from_column_vec(n, 1, (0..n).map(|i| i as f64).collect()).unwrap();
                let mut e = Engine::default();
                e.variables.insert("A".into(), source.clone());
                for pattern in 0..4 {
                    let chosen = |i: usize| match pattern {
                        0 => true,
                        1 => false,
                        2 => i >= 8191 && i < n - 8193,
                        _ => i.is_multiple_of(2),
                    };
                    let mask = matrust::LogicalMatrix::from_column_vec(
                        n,
                        1,
                        (0..n).map(|i| u8::from(chosen(i))).collect(),
                        &flag,
                    )
                    .unwrap();
                    e.logical_variables.insert("M".into(), mask);
                    e.execute("R=A(M);").unwrap();
                    let expected: Vec<_> =
                        (0..n).filter(|i| chosen(*i)).map(|i| i as f64).collect();
                    assert_eq!(e.variables["R"].data(), expected.as_slice());
                    if pattern == 0 {
                        assert_eq!(e.variables["R"].data().as_ptr(), source.data().as_ptr());
                    }
                    e.execute("M(M)=false;").unwrap();
                    assert!(e.logical_variables["M"].data().iter().all(|v| *v == 0));
                }
                e.execute("A(1:8192)=true;A(end)=false;").unwrap();
                assert_eq!(source.data()[0], 0.);
                assert_eq!(e.variables["A"].data()[0], 1.);
                assert_eq!(e.variables["A"].data()[n - 1], 0.);
            });
    }
}
