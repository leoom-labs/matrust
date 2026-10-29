//! Release draft: output allocation/computation included, destruction excluded.
//! Pools, immutable inputs, promoted serial controls and checks are untimed.
use matrust::{Array, Complex64, ComplexMatrix, Matrix, complex_matrix::ComplexOp};
use nalgebra::DMatrix;
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn input(rows: usize, cols: usize, complex: bool, seed: usize) -> Array {
    let re: Vec<_> = (0..rows * cols)
        .map(|i| ((i + seed) % 17) as f64 / 16. + 0.25)
        .collect();
    if complex {
        ComplexMatrix::from_column_vec(
            rows,
            cols,
            re.into_iter()
                .enumerate()
                .map(|(i, re)| Complex64::new(re, ((i + seed) % 11) as f64 / 8. - 0.5))
                .collect(),
            &AtomicBool::new(false),
        )
        .unwrap()
        .into()
    } else {
        Matrix::from_column_vec(rows, cols, re).unwrap().into()
    }
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let reverse = std::env::args().skip(1).any(|x| x == "--reverse");
    let cancel = AtomicBool::new(false);
    let mut workers = vec![1, 8];
    let mut modes = vec!["cc", "rc"];
    let mut shapes = vec![(1, 1), (8193, 1), (1024, 64), (64, 8193), (1048576, 1)];
    let mut operators = vec![
        ("add", ComplexOp::Add),
        ("subtract", ComplexOp::Subtract),
        ("multiply", ComplexOp::Multiply),
        ("divide", ComplexOp::Divide),
    ];
    let mut products = vec![
        (8, 8, 8),
        (129, 129, 33),
        (256, 256, 256),
        (512, 512, 512),
        (1024, 64, 64),
        (64, 1024, 64),
    ];
    let mut implementations = vec!["candidate", "serial_nalgebra_control"];
    if reverse {
        workers.reverse();
        modes.reverse();
        shapes.reverse();
        operators.reverse();
        products.reverse();
        implementations.reverse();
    }
    println!("workers,rows,inner,cols,mode,operation,implementation,sample,microseconds");
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for &mode in &modes {
            for &(rows, cols) in &shapes {
                let a = input(rows, cols, mode == "cc", 3);
                let b = input(rows, cols, true, 7);
                for &(operation, op) in &operators {
                    let expected = pool
                        .install(|| ComplexMatrix::arithmetic(&a, &b, op, &cancel))
                        .unwrap();
                    for iteration in 0..9 {
                        let start = Instant::now();
                        let output = pool
                            .install(|| {
                                ComplexMatrix::arithmetic(black_box(&a), black_box(&b), op, &cancel)
                            })
                            .unwrap();
                        let elapsed = start.elapsed();
                        assert_eq!(output.data(), expected.data());
                        black_box(output.data());
                        if iteration >= 2 {
                            println!(
                                "{workers},{rows},0,{cols},{mode},{operation},candidate,{},{:.6}",
                                iteration - 2,
                                elapsed.as_secs_f64() * 1e6
                            );
                        }
                    }
                }
            }
            for &(m, k, n) in &products {
                let a = input(m, k, mode == "cc", 3);
                let b = input(k, n, true, 7);
                // Compare a promoted serial library control; it
                // excludes mixed conversion unlike the candidate's bounded packs.
                let ca = DMatrix::from_vec(
                    m,
                    k,
                    (0..k)
                        .flat_map(|c| (0..m).map(move |r| (r, c)))
                        .map(|(r, c)| a.scalar(r, c).unwrap().into_complex())
                        .collect(),
                );
                let cb = DMatrix::from_vec(k, n, b.as_complex().unwrap().data().to_vec());
                let mut reference = DMatrix::zeros(m, n);
                reference.gemm(Complex64::new(1., 0.), &ca, &cb, Complex64::new(0., 0.));
                for &implementation in &implementations {
                    for iteration in 0..9 {
                        let start = Instant::now();
                        let result = pool.install(|| {
                            if implementation == "candidate" {
                                ComplexMatrix::arithmetic(
                                    black_box(&a),
                                    black_box(&b),
                                    ComplexOp::Matmul,
                                    &cancel,
                                )
                                .unwrap()
                            } else {
                                let mut out = DMatrix::zeros(m, n);
                                out.gemm(
                                    Complex64::new(1., 0.),
                                    black_box(&ca),
                                    black_box(&cb),
                                    Complex64::new(0., 0.),
                                );
                                // Move the control result without a full payload copy.
                                ComplexMatrix::from_column_vec(m, n, out.data.into(), &cancel)
                                    .unwrap()
                            }
                        });
                        let elapsed = start.elapsed();
                        for (x, y) in result.data().iter().zip(reference.as_slice()) {
                            assert!((x.re - y.re).abs() <= 1e-11 * (1. + y.re.abs()));
                            assert!((x.im - y.im).abs() <= 1e-11 * (1. + y.im.abs()));
                        }
                        black_box(result.data());
                        if iteration >= 2 {
                            println!(
                                "{workers},{m},{k},{n},{mode},matmul,{implementation},{},{:.6}",
                                iteration - 2,
                                elapsed.as_secs_f64() * 1e6
                            );
                        }
                    }
                }
            }
        }
    }
}
