use matrust::{Array, Engine, Matrix, files};
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
        "A()",
        "A(1:50331649)",
    ] {
        assert!(e.execute(&format!("B={expr};")).is_err(), "{expr}");
        assert_eq!(e.variables["B"].data(), &[17.]);
        assert_eq!(e.variables["A"].data().as_ptr(), source.data().as_ptr());
    }
    // review_d1112: 1:0:3 is a 1-by-0 empty (Octave 10.3), so A(1:0:3) is an empty
    // index, not an error. The old test required this call to fail.
    e.execute("E=A(1:0:3);").unwrap();
    assert_eq!((e.variables["E"].rows(), e.variables["E"].cols()), (1, 0));
    assert!(e.variables["E"].data().is_empty());
    assert_eq!(e.variables["B"].data(), &[17.]);
    assert_eq!(e.variables["A"].data().as_ptr(), source.data().as_ptr());
    e.execute("C=A(1,1,1);").unwrap();
    assert_eq!(e.variables["C"].data(), &[1.]);
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

/// Octave 10.3 (`tool/ref-octave.sh`) and `colon`, which already matched it.
/// `1:0:3` and `0:0:0` are 1-by-0; `NaN:5` and `Inf:Inf` are 1-by-1 NaN;
/// `1:Inf:2` is `[1]`; `1:Inf` errors. MATLAB Answers and the R2016a colon
/// oracle agree on the zero step, NaN, and `colon(1,Inf)`.
#[test]
fn review_d1112_colon_zero_step_nan_and_inf_match_octave() {
    let mut e = Engine::default();
    e.execute(
        "Z=1:0:3; Y=colon(1,0,3); N=NaN:5; M=colon(NaN,5); W=NaN:0:1; O=0:0:0; \
         E=1:Inf:2; F=colon(1,Inf,2); G=Inf:Inf; H=1:-Inf:0; I=Inf:5; J=2:Inf:5; \
         K=1:0:Inf; L=Inf:1:Inf; P=5:-Inf:1; Q=colon(Inf,5);",
    )
    .unwrap();
    for name in ["Z", "Y", "O", "I", "K", "Q"] {
        let v = &e.variables[name];
        assert_eq!((v.rows(), v.cols(), v.data().len()), (1, 0, 0), "{name}");
    }
    for name in ["N", "M", "W", "G", "L"] {
        let v = &e.variables[name];
        assert_eq!((v.rows(), v.cols()), (1, 1), "{name}");
        assert!(v.data()[0].is_nan(), "{name}");
    }
    assert_eq!(e.variables["E"].data(), &[1.]);
    assert_eq!(e.variables["F"].data(), &[1.]);
    assert_eq!(e.variables["H"].data(), &[1.]);
    assert_eq!(e.variables["J"].data(), &[2.]);
    assert_eq!(e.variables["P"].data(), &[5.]);
    for expr in ["1:Inf", "colon(1,Inf)", "-Inf:1", "1:-1:-Inf", "Inf:-1:0"] {
        let err = e.execute(&format!("X={expr};")).unwrap_err();
        assert!(
            err.contains("Maximum variable size allowed by the program is exceeded."),
            "{expr}: {err}"
        );
    }
    e.execute("R='a':0:'c'; S=colon('a',0,'c');").unwrap();
    for name in ["R", "S"] {
        let a = e.array(name).unwrap();
        assert_eq!(a.class(), "char", "{name}");
        assert_eq!(a.dims().as_ref(), &[1, 0], "{name}");
    }
}

/// Octave 10.3: a vector indexed by a non-vector logical mask keeps the
/// vector's orientation. A matrix source stays a column. MathWorks logical
/// indexing: the result is a column unless the indexed array is a row vector.
#[test]
fn review_d1112_vector_logical_mask_keeps_source_orientation() {
    let mut e = Engine::default();
    e.execute(
        "A=[10 20 30 40]; R=A(logical([1 0; 0 1])); C=A(logical([1 0; 1 0])); \
         V=A(logical([1 0 0 1])); U=A(logical([1;0;0;1])); Z=A(logical(zeros(2,2))); \
         B=[10;20;30;40]; D=B(logical([1 0; 0 1])); E=B(logical(zeros(2,2))); \
         M=[10 20; 30 40]; N=M(logical([1 0; 0 1])); F=M(logical(zeros(2,2))); \
         S=sparse([10 20 30 40]); T=S(logical([1 0; 0 1])); \
         P=sparse([10;20;30;40]); Q=P(logical([1 0; 0 1])); \
         G=sparse([10 20; 30 40]); H=G(logical([1 0; 0 1]));",
    )
    .unwrap();
    let shape = |e: &Engine, name: &str| {
        let v = &e.variables[name];
        (v.rows(), v.cols(), v.data().to_vec())
    };
    assert_eq!(shape(&e, "R"), (1, 2, vec![10., 40.]));
    assert_eq!(shape(&e, "C"), (1, 2, vec![10., 20.]));
    assert_eq!(shape(&e, "V"), (1, 2, vec![10., 40.]));
    assert_eq!(shape(&e, "U"), (1, 2, vec![10., 40.]));
    assert_eq!(shape(&e, "Z"), (1, 0, vec![]));
    assert_eq!(shape(&e, "D"), (2, 1, vec![10., 40.]));
    assert_eq!(shape(&e, "E"), (0, 1, vec![]));
    assert_eq!(shape(&e, "N"), (2, 1, vec![10., 40.]));
    assert_eq!(shape(&e, "F"), (0, 1, vec![]));
    let sparse_full = |e: &Engine, name: &str| {
        let Some(Array::Sparse(s)) = e.array(name) else {
            panic!("{name} is not sparse");
        };
        let dense = s
            .to_dense(&std::sync::atomic::AtomicBool::new(false))
            .unwrap();
        (s.rows(), s.cols(), dense.data().to_vec())
    };
    assert_eq!(sparse_full(&e, "T"), (1, 2, vec![10., 40.]));
    assert_eq!(sparse_full(&e, "Q"), (2, 1, vec![10., 40.]));
    assert_eq!(sparse_full(&e, "H"), (2, 1, vec![10., 40.]));
}
