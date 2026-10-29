use matrust::{Array, Complex64, ComplexMatrix, LogicalMatrix, Matrix, complex_matrix::ComplexOp};
use std::sync::atomic::AtomicBool;

#[test]
fn tiny_mixed_gemm_reports_only_required_owned_panels() {
    let cancel = AtomicBool::new(false);
    let real: Array = Matrix::filled(8, 8, 2.).unwrap().into();
    let complex: Array = ComplexMatrix::filled(8, 8, Complex64::new(3., -4.), &cancel)
        .unwrap()
        .into();
    assert_eq!(
        ComplexMatrix::arithmetic_scratch_bytes(&real, &complex, ComplexOp::Matmul),
        1024
    );
    assert_eq!(
        ComplexMatrix::arithmetic_scratch_bytes(&complex, &real, ComplexOp::Matmul),
        1024
    );
    assert_eq!(
        ComplexMatrix::arithmetic_scratch_bytes(&complex, &complex, ComplexOp::Matmul),
        0
    );
}

#[test]
fn mixed_gemm_panel_tails_and_parallel_job_tails_preserve_exact_products() {
    let cancel = AtomicBool::new(false);
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        // Last shape exceeds the scheduling threshold and has an uneven job tail.
        for (m, k, n) in [(8, 8, 8), (3, 129, 35), (129, 7, 33), (129, 257, 257)] {
            let real: Array = Matrix::filled(m, k, 2.).unwrap().into();
            let right: Array = ComplexMatrix::filled(k, n, Complex64::new(3., -4.), &cancel)
                .unwrap()
                .into();
            let left: Array = ComplexMatrix::filled(m, k, Complex64::new(5., 2.), &cancel)
                .unwrap()
                .into();
            let logical: Array = LogicalMatrix::from_column_vec(k, n, vec![1; k * n], &cancel)
                .unwrap()
                .into();
            for (a, b, expected) in [
                (&real, &right, Complex64::new(6. * k as f64, -8. * k as f64)),
                (
                    &left,
                    &logical,
                    Complex64::new(5. * k as f64, 2. * k as f64),
                ),
            ] {
                let aliases = (a.clone(), b.clone());
                let result = pool
                    .install(|| ComplexMatrix::arithmetic(a, b, ComplexOp::Matmul, &cancel))
                    .unwrap();
                assert_eq!((result.rows(), result.cols()), (m, n));
                assert!(result.data().iter().all(|&x| x == expected));
                assert!(a.same_snapshot(&aliases.0) && b.same_snapshot(&aliases.1));
            }
        }
    }
}
