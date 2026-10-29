//! Script allocation/metadata costs. Constructors remain serial pending evidence;
//! one/eight configured pools measure context overhead, not promised scaling.
use matrust::{Complex64, ComplexMatrix, Engine, LogicalMatrix, Matrix};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(
        args.iter()
            .all(|a| a == "--reverse" || a == "--legacy-only")
    );
    let reverse = args.iter().any(|a| a == "--reverse");
    let legacy = args.iter().any(|a| a == "--legacy-only");
    let mut workers = vec![1, 8];
    let mut shapes = vec![
        (0, 7),
        (1, 1),
        (4095, 1),
        (4096, 1),
        (4097, 1),
        (8193, 1),
        (1024, 64),
        (64, 8193),
        (1048576, 1),
    ];
    if reverse {
        workers.reverse();
        shapes.reverse();
    }
    println!("workers,rows,cols,operation,sample,microseconds");
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for &(rows, cols) in &shapes {
            let a = Matrix::filled(rows, cols, 3.).unwrap();
            let cancel = AtomicBool::new(false);
            let l =
                LogicalMatrix::from_column_vec(rows, cols, vec![1; rows * cols], &cancel).unwrap();
            let z = ComplexMatrix::from_column_vec(
                rows,
                cols,
                vec![Complex64::new(3., -3.); rows * cols],
                &cancel,
            )
            .unwrap();
            let mut jobs: Vec<(&str, String, f64)> = vec![
                ("zeros_scalar", format!("R=zeros({rows},{cols});"), 0.),
                ("ones_scalar", format!("R=ones({rows},{cols});"), 1.),
                ("reshape_scalar", format!("R=reshape(A,{rows},{cols});"), 3.),
                ("logical_scalar", format!("R=reshape(L,{rows},{cols});"), 1.),
                ("complex_scalar", format!("R=reshape(Z,{rows},{cols});"), 3.),
            ];
            if !legacy {
                jobs.extend([
                    ("zeros_size", "R=zeros(size(A));".into(), 0.),
                    ("ones_size", "R=ones(size(A));".into(), 1.),
                    ("reshape_vector", "R=reshape(A,size(A));".into(), 3.),
                    ("reshape_inferred", format!("R=reshape(A,[],{cols});"), 3.),
                    ("logical_vector", "R=reshape(L,size(L));".into(), 1.),
                    ("logical_inferred", format!("R=reshape(L,[],{cols});"), 1.),
                    ("complex_vector", "R=reshape(Z,size(Z));".into(), 3.),
                    ("complex_inferred", format!("R=reshape(Z,[],{cols});"), 3.),
                ]);
            }
            if reverse {
                jobs.reverse();
            }
            for (operation, script, value) in jobs {
                let mut e = Engine::default();
                e.variables.insert("A".into(), a.clone());
                e.logical_variables.insert("L".into(), l.clone());
                e.complex_variables.insert("Z".into(), z.clone());
                for iteration in 0..9 {
                    // Drop prior output outside timing; source remains shared.
                    e.remove_array("R");
                    let start = Instant::now();
                    let result = pool.install(|| e.execute(black_box(&script)));
                    let elapsed = start.elapsed();
                    result.unwrap();
                    if operation.starts_with("logical") {
                        let out = &e.logical_variables["R"];
                        assert_eq!((out.rows(), out.cols()), (rows, cols));
                        assert_eq!(out.data().as_ptr(), l.data().as_ptr());
                    } else if operation.starts_with("complex") {
                        let out = &e.complex_variables["R"];
                        assert_eq!((out.rows(), out.cols()), (rows, cols));
                        assert!(out.shares_storage(&z));
                    } else {
                        let out = &e.variables["R"];
                        assert_eq!((out.rows(), out.cols()), (rows, cols));
                        assert!(out.data().iter().all(|v| v.to_bits() == value.to_bits()));
                        if operation.starts_with("reshape") {
                            assert_eq!(out.data().as_ptr(), a.data().as_ptr());
                        }
                    }
                    if iteration >= 2 {
                        println!(
                            "{workers},{rows},{cols},{operation},{},{:.6}",
                            iteration - 2,
                            elapsed.as_secs_f64() * 1e6
                        );
                    }
                }
            }
        }
    }
}
