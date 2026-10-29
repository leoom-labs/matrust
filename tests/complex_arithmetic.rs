use matrust::{
    Array, Complex64, ComplexMatrix, Engine, LogicalMatrix, Matrix, complex_matrix::ComplexOp,
};
use std::sync::atomic::{AtomicBool, Ordering};
fn array(rows: usize, cols: usize, offset: usize) -> Array {
    ComplexMatrix::from_column_vec(
        rows,
        cols,
        (0..rows * cols)
            .map(|i| {
                Complex64::new(
                    ((i + offset) % 11) as f64 - 5.,
                    ((i * 3 + offset) % 7) as f64 - 3.,
                )
            })
            .collect(),
        &AtomicBool::new(false),
    )
    .unwrap()
    .into()
}
fn compare(actual: &ComplexMatrix, expected: &[(f64, f64)]) {
    assert_eq!(actual.len(), expected.len());
    for (a, &(r, i)) in actual.data().iter().zip(expected) {
        assert!(
            (a.re - r).abs() <= 2e-13 * (1. + r.abs()),
            "real {a:?} vs {r}"
        );
        assert!(
            (a.im - i).abs() <= 2e-13 * (1. + i.abs()),
            "imag {a:?} vs {i}"
        );
    }
}

#[test]
fn complex_literal_suffixes_precedence_shadowing_and_transposes() {
    let mut e = Engine::default();
    e.execute("i=7;j=9;").unwrap();
    // i() is ordinary indexing when i is a workspace variable.
    assert!(e.execute("G=i();").is_err());
    e.execute("A=1+2i;B=3-4j;C=i+j;D=[1i 2j;3i -4j];T=D.';H=D';E=-2e-3i;F=.5j;")
        .unwrap();
    assert_eq!(e.variables["C"].data(), &[16.]);
    compare(&e.complex_variables["A"], &[(1., 2.)]);
    compare(&e.complex_variables["B"], &[(3., -4.)]);
    compare(
        &e.complex_variables["D"],
        &[(0., 1.), (0., 3.), (0., 2.), (0., -4.)],
    );
    compare(
        &e.complex_variables["T"],
        &[(0., 1.), (0., 2.), (0., 3.), (0., -4.)],
    );
    compare(
        &e.complex_variables["H"],
        &[(0., -1.), (0., -2.), (0., -3.), (0., 4.)],
    );
    compare(&e.complex_variables["E"], &[(0., -0.002)]);
    compare(&e.complex_variables["F"], &[(0., 0.5)]);
    e.execute("clear i j;U=i;V=j();W=2i';X=2j.';").unwrap();
    compare(&e.complex_variables["U"], &[(0., 1.)]);
    compare(&e.complex_variables["V"], &[(0., 1.)]);
    compare(&e.complex_variables["W"], &[(0., -2.)]);
    compare(&e.complex_variables["X"], &[(0., 2.)]);
    for bad in ["X=2ii;", "X=2j3;", "X=2i_name;", "X=2 i;"] {
        assert!(e.execute(bad).is_err(), "{bad}");
    }
    let mut functions = Engine::default();
    functions
        .execute("A=i;B=i();C=1i;function y=i();y=6;end")
        .unwrap();
    assert_eq!(functions.variables["A"].data(), &[6.]);
    assert_eq!(functions.variables["B"].data(), &[6.]);
    compare(&functions.complex_variables["C"], &[(0., 1.)]);
}

#[test]
fn complex_script_arithmetic_mixed_types_scalar_expansion_and_real_paths() {
    let mut e = Engine::default();
    e.execute("Z=[1+2i 3-4i];Alias=Z;A=Z+2;B=2-Z;C=Z.*[2 3];D=Z./[1-2i 2];E=true+Z;F=Z*2;R=[1 2]+[3 4];L=(1:2)>1;S=L.*[3 4];").unwrap();
    compare(&e.complex_variables["A"], &[(3., 2.), (5., -4.)]);
    compare(&e.complex_variables["B"], &[(1., -2.), (-1., 4.)]);
    compare(&e.complex_variables["C"], &[(2., 4.), (9., -12.)]);
    compare(&e.complex_variables["D"], &[(-0.6, 0.8), (1.5, -2.)]);
    compare(&e.complex_variables["E"], &[(2., 2.), (4., -4.)]);
    compare(&e.complex_variables["F"], &[(2., 4.), (6., -8.)]);
    assert!(e.complex_variables["Z"].shares_storage(&e.complex_variables["Alias"]));
    assert_eq!(e.variables["R"].data(), &[4., 6.]);
    assert_eq!(e.variables["S"].data(), &[0., 4.]);
    let before = e.workspace();
    // Complex powers, scalar division and least-squares solves are supported; a non-square matrix power still fails.
    e.execute("P=Z.^2;").unwrap();
    compare(&e.complex_variables["P"], &[(-3., 4.), (-7., -24.)]);
    e.execute("clear P").unwrap();
    for bad in ["Z=Z^2;", "Z=Z+[1;2];"] {
        assert!(e.execute(bad).is_err());
        for (name, value) in &before {
            assert!(value.same_snapshot(&e.array(name).unwrap()), "{bad}:{name}");
        }
    }
    // Unsupported left operands still fail before the right side runs.
    assert!(
        e.execute("R={}+effect();function y=effect();disp('MUST_NOT_RUN');y=1;end")
            .is_err()
    );
    assert!(e.failed_output.output.is_empty());
    // Char is numeric in arithmetic (MATLAB): its codes plus the right side.
    e.execute("R='text'+one();function y=one();y=1;end")
        .unwrap();
    assert_eq!(e.variables["R"].data(), &[117., 102., 121., 117.]);
}

#[test]
fn complex_gemm_rectangles_reshape_mixed_inputs_and_tile_tails() {
    let cancel = AtomicBool::new(false);
    for (m, k, n) in [(1, 7, 1), (3, 7, 4), (129, 129, 33), (7, 257, 35)] {
        let a = array(m, k, 2);
        let b = array(k, n, 5);
        let mut expected = Vec::new();
        // Independent real/imag reference expressions, used only for modest
        // integer fixtures whose products/sums are exactly representable.
        for c in 0..n {
            for r in 0..m {
                let mut re = 0.;
                let mut im = 0.;
                for depth in 0..k {
                    let x = a.scalar(r, depth).unwrap().into_complex();
                    let y = b.scalar(depth, c).unwrap().into_complex();
                    re += x.re * y.re - x.im * y.im;
                    im += x.re * y.im + x.im * y.re;
                }
                expected.push((re, im));
            }
        }
        for workers in [1, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            let output = pool
                .install(|| ComplexMatrix::arithmetic(&a, &b, ComplexOp::Matmul, &cancel))
                .unwrap();
            compare(&output, &expected);
        }
        // Reshape must not let the allocation's original stride leak into GEMM.
        let aa = a
            .as_complex()
            .unwrap()
            .reshape(1, m * k, &cancel)
            .unwrap()
            .reshape(m, k, &cancel)
            .unwrap()
            .into();
        compare(
            &ComplexMatrix::arithmetic(&aa, &b, ComplexOp::Matmul, &cancel).unwrap(),
            &expected,
        );
    }
    let real: Array = Matrix::from_column_slice(2, 2, &[1., 3., 2., 4.])
        .unwrap()
        .into();
    let complex: Array = ComplexMatrix::from_column_vec(
        2,
        1,
        vec![Complex64::new(1., 2.), Complex64::new(3., -1.)],
        &cancel,
    )
    .unwrap()
    .into();
    compare(
        &ComplexMatrix::arithmetic(&real, &complex, ComplexOp::Matmul, &cancel).unwrap(),
        &[(7., 0.), (15., 2.)],
    );
    let logical: Array = LogicalMatrix::from_column_vec(2, 2, vec![1, 0, 0, 1], &cancel)
        .unwrap()
        .into();
    compare(
        &ComplexMatrix::arithmetic(&logical, &complex, ComplexOp::Matmul, &cancel).unwrap(),
        &[(1., 2.), (3., -1.)],
    );
    let complex_row: Array = complex
        .as_complex()
        .unwrap()
        .reshape(1, 2, &cancel)
        .unwrap()
        .into();
    compare(
        &ComplexMatrix::arithmetic(&complex_row, &real, ComplexOp::Matmul, &cancel).unwrap(),
        &[(10., -1.), (14., 0.)],
    );
    compare(
        &ComplexMatrix::arithmetic(&complex_row, &logical, ComplexOp::Matmul, &cancel).unwrap(),
        &[(1., 2.), (3., -1.)],
    );
    // Cross the provisional GEMM scheduling threshold without using a large
    // scalar-loop reference: this exact constant product has a closed-form result.
    let a: Array = ComplexMatrix::filled(257, 129, Complex64::new(1., 2.), &cancel)
        .unwrap()
        .into();
    let b: Array = ComplexMatrix::filled(129, 257, Complex64::new(2., -1.), &cancel)
        .unwrap()
        .into();
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let product = pool
            .install(|| ComplexMatrix::arithmetic(&a, &b, ComplexOp::Matmul, &cancel))
            .unwrap();
        assert!(
            product
                .data()
                .iter()
                .all(|z| *z == Complex64::new(516., 387.))
        );
    }
}

#[test]
fn complex_arithmetic_empty_shape_bounds_pre_cancel_and_parallel_parity() {
    let cancel = AtomicBool::new(false);
    for (m, k, n) in [(0, 7, 3), (3, 0, 7), (3, 7, 0), (0, 0, 0)] {
        let a = array(m, k, 0);
        let b = array(k, n, 0);
        let c = ComplexMatrix::arithmetic(&a, &b, ComplexOp::Matmul, &cancel).unwrap();
        assert_eq!((c.rows(), c.cols()), (m, n));
        assert!(c.data().iter().all(|x| *x == Complex64::new(0., 0.)));
    }
    let empty = array(0, 3, 0);
    let scalar = array(1, 1, 2);
    let expanded = ComplexMatrix::arithmetic(&empty, &scalar, ComplexOp::Add, &cancel).unwrap();
    assert_eq!((expanded.rows(), expanded.cols()), (0, 3));
    let a = array(1024, 513, 1);
    let b = array(1024, 513, 3);
    let aliases = (a.clone(), b.clone());
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    for op in [
        ComplexOp::Add,
        ComplexOp::Subtract,
        ComplexOp::Multiply,
        ComplexOp::Divide,
    ] {
        let x = pools[0]
            .install(|| ComplexMatrix::arithmetic(&a, &b, op, &cancel))
            .unwrap();
        let y = pools[1]
            .install(|| ComplexMatrix::arithmetic(&a, &b, op, &cancel))
            .unwrap();
        assert!(
            x.data()
                .iter()
                .zip(y.data())
                .all(|(a, b)| a.re.to_bits() == b.re.to_bits() && a.im.to_bits() == b.im.to_bits())
        );
    }
    cancel.store(true, Ordering::Relaxed);
    assert!(ComplexMatrix::arithmetic(&a, &b, ComplexOp::Add, &cancel).is_err());
    assert!(a.same_snapshot(&aliases.0) && b.same_snapshot(&aliases.1));
    assert!(ComplexMatrix::checked_len(usize::MAX, 2).is_err());
    let big = ComplexMatrix::filled(
        8_388_608,
        0,
        Complex64::new(0., 0.),
        &AtomicBool::new(false),
    )
    .unwrap()
    .into();
    let zero = ComplexMatrix::filled(0, 2, Complex64::new(0., 0.), &AtomicBool::new(false))
        .unwrap()
        .into();
    assert!(
        ComplexMatrix::arithmetic(&big, &zero, ComplexOp::Matmul, &AtomicBool::new(false)).is_err()
    );
}
