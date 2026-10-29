use matrust::{Engine, Matrix, files};
use std::path::Path;

#[test]
fn indexing_matches_matlab_values_and_shapes() {
    let expected =
        files::load_workspace(Path::new("tests/fixtures/indexing/indexing-oracle.mat")).unwrap();
    assert_eq!(expected.len(), 33);
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let mut engine = Engine::default();
                engine
                    .execute(include_str!("fixtures/indexing/indexing_cases.m"))
                    .unwrap();
                for (name, reference) in &expected {
                    let actual = &engine.variables[name];
                    assert_eq!(
                        (actual.rows(), actual.cols()),
                        (reference.rows(), reference.cols()),
                        "{name}"
                    );
                    assert_eq!(actual.data(), reference.data(), "{name}");
                }
            });
    }
}

#[test]
fn invalid_subscripts_preserve_existing_output_and_source() {
    let mut e = Engine::default();
    e.execute("A=reshape(1:24,4,6); B=17;").unwrap();
    let source = e.variables["A"].clone();
    for expr in [
        "A(0:2)",
        "A(end:-1:0)",
        "A(1:25)",
        "A(1:0.5:3)",
        "A(NaN)",
        "A(Inf)",
        "A([1 25])",
        "A(:,7)",
        "A(5,:)",
        "A(1:0:3)",
        "A(1,1,1)",
        "A()",
        "A(1:16777217)",
    ] {
        assert!(e.execute(&format!("B={expr};")).is_err(), "{expr}");
        assert_eq!(e.variables["B"].data(), &[17.]);
        assert_eq!(e.variables["A"].data().as_ptr(), source.data().as_ptr());
    }
    e.cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(e.execute("B=A(:);").unwrap_err().contains("cancelled"));
    assert_eq!(e.variables["B"].data(), &[17.]);
}

#[test]
fn identity_slices_share_data_and_edits_preserve_aliases() {
    let mut e = Engine::default();
    e.execute("A=reshape(1:24,4,6); B=A(:,:); C=A(1:end,1:end);")
        .unwrap();
    assert_eq!(
        e.variables["A"].data().as_ptr(),
        e.variables["B"].data().as_ptr()
    );
    assert_eq!(
        e.variables["A"].data().as_ptr(),
        e.variables["C"].data().as_ptr()
    );
    let changed = e.variables["B"].with_value(0, 0, 99.).unwrap();
    e.variables.insert("B".into(), changed);
    assert_eq!(e.variables["A"].get(0, 0), 1.);
    assert_eq!(e.variables["C"].get(0, 0), 1.);
    e.execute("v=[1;2;3]; w=v(:);").unwrap();
    assert_eq!(
        e.variables["v"].data().as_ptr(),
        e.variables["w"].data().as_ptr()
    );
    e.execute("flat=A(:); reshaped=reshape(A,3,8);").unwrap();
    assert_eq!(
        e.variables["A"].data().as_ptr(),
        e.variables["flat"].data().as_ptr()
    );
    assert_eq!(
        e.variables["A"].data().as_ptr(),
        e.variables["reshaped"].data().as_ptr()
    );
    assert_eq!(
        (e.variables["flat"].rows(), e.variables["flat"].cols()),
        (24, 1)
    );
    assert_eq!(
        (
            e.variables["reshaped"].rows(),
            e.variables["reshaped"].cols()
        ),
        (3, 8)
    );
}

#[test]
fn reshaped_storage_uses_logical_shape_in_numerical_kernels_and_files() {
    fn same(a: &Matrix, b: &Matrix) {
        assert_eq!((a.rows(), a.cols()), (b.rows(), b.cols()));
        assert_eq!(a.data(), b.data());
    }
    let storage = Matrix::from_column_slice(1, 9, &[4., 1., 2., 1., 5., 1., 2., 1., 6.]).unwrap();
    let shaped = storage.reshape(3, 3).unwrap();
    let owned = Matrix::from_column_slice(3, 3, storage.data()).unwrap();
    let rhs = Matrix::from_column_slice(1, 6, &[1., 2., 3., 4., 5., 6.])
        .unwrap()
        .reshape(3, 2)
        .unwrap();
    same(&shaped.transpose(), &owned.transpose());
    same(&shaped.mul(&rhs).unwrap(), &owned.mul(&rhs).unwrap());
    same(&shaped.solve(&rhs).unwrap(), &owned.solve(&rhs).unwrap());
    same(&shaped.inverse().unwrap(), &owned.inverse().unwrap());
    same(&shaped.det().unwrap(), &owned.det().unwrap());
    same(&shaped.power(3.).unwrap(), &owned.power(3.).unwrap());
    same(
        &shaped.reduce(false).unwrap(),
        &owned.reduce(false).unwrap(),
    );
    same(&shaped.reduce(true).unwrap(), &owned.reduce(true).unwrap());
    same(&shaped.sin(), &owned.sin());
    same(
        &shaped.zip(&owned, |a, b| a + b).unwrap(),
        &owned.map(|a| 2. * a),
    );
    same(
        &Matrix::concatenate(vec![vec![shaped.clone(), rhs.clone()]]).unwrap(),
        &Matrix::concatenate(vec![vec![owned.clone(), rhs.clone()]]).unwrap(),
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shaped.mat");
    let vars = [("A".to_owned(), shaped.clone())].into_iter().collect();
    files::save_workspace(&path, &vars).unwrap();
    same(&files::load_workspace(&path).unwrap()["A"], &owned);
    assert_eq!(shaped.with_value(2, 1, 99.).unwrap().get(2, 1), 99.);
    assert_eq!(storage.data()[5], 1.);
    assert!(storage.reshape(2, 5).is_err());
    assert!(storage.reshape(usize::MAX, 2).is_err());
    same(
        &Matrix::filled(0, 7, 0.).unwrap().reshape(3, 0).unwrap(),
        &Matrix::filled(3, 0, 0.).unwrap(),
    );
    // Exercise both the packed GEMM backend and the shallow-inner parallel fallback.
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                for (m, k, n) in [(96, 96, 96), (512, 32, 512)] {
                    let a = Matrix::from_column_vec(
                        1,
                        m * k,
                        (0..m * k).map(|i| (i % 29) as f64 / 29.).collect(),
                    )
                    .unwrap();
                    let b = Matrix::from_column_vec(
                        1,
                        k * n,
                        (0..k * n).map(|i| (i % 17) as f64 / 17.).collect(),
                    )
                    .unwrap();
                    let av = a.reshape(m, k).unwrap();
                    let bv = b.reshape(k, n).unwrap();
                    let ac = Matrix::from_column_slice(m, k, a.data()).unwrap();
                    let bc = Matrix::from_column_slice(k, n, b.data()).unwrap();
                    same(&av.mul(&bv).unwrap(), &ac.mul(&bc).unwrap());
                }
            });
    }
}

#[test]
fn large_compact_slices_and_explicit_gathers_match_reference() {
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let rows = 524288;
                let mut e = Engine::default();
                e.variables.insert(
                    "A".into(),
                    Matrix::from_column_vec(rows, 8, (0..rows * 8).map(|i| i as f64).collect())
                        .unwrap(),
                );
                for script in ["B=A(2:3:end,8:-2:1);", "I=2:3:524288; B=A(I,[8 6 4 2]);"] {
                    e.execute(script).unwrap();
                    let b = &e.variables["B"];
                    let expected: Vec<_> = [7, 5, 3, 1]
                        .into_iter()
                        .flat_map(|c| (1..rows).step_by(3).map(move |r| (c * rows + r) as f64))
                        .collect();
                    assert_eq!(b.data(), expected);
                    assert_eq!(b.cols(), 4);
                }
                e.execute("B=A(end:-1:1);").unwrap();
                for (i, &v) in e.variables["B"].data().iter().enumerate() {
                    assert_eq!(v, (rows * 8 - i - 1) as f64);
                }
            });
    }
}
