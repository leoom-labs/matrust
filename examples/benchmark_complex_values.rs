//! End-to-end script costs; setup/validation are untimed. Existing shared source
//! aliases survive each sample; statement result replacement/drop is timed.
use matrust::{Complex64, ComplexMatrix, Engine};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let reverse = std::env::args().skip(1).any(|x| x == "--reverse");
    let mut shapes = vec![
        (1, 1),
        (8193, 1),
        (1, 8193),
        (1024, 64),
        (64, 8193),
        (1048576, 1),
    ];
    let mut workers = vec![1, 8];
    if reverse {
        shapes.reverse();
        workers.reverse();
    }
    println!("workers,rows,cols,operation,sample,microseconds");
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for &(rows, cols) in &shapes {
            let source = ComplexMatrix::from_column_vec(
                rows,
                cols,
                (0..rows * cols)
                    .map(|i| Complex64::new((i % 997) as f64 / 997., -((i % 251) as f64) / 251.))
                    .collect(),
                &AtomicBool::new(false),
            )
            .unwrap();
            let mut jobs = vec![
                ("components", "R=real(Z);I=imag(Z);"),
                ("adjoint", "Y=Z';"),
                ("gather", "Y=Z(1:2:end);"),
                ("indexed_write", "Z(1)=complex(3,4);"),
                ("shared_assign_clear", "B=Z;clear B;"),
            ];
            if reverse {
                jobs.reverse();
            }
            for &(operation, script) in &jobs {
                let mut e = Engine::default();
                e.complex_variables.insert("source".into(), source.clone());
                for iteration in 0..9 {
                    // Reset typed maps outside the timer; source remains shared.
                    e.remove_array("R");
                    e.remove_array("I");
                    e.remove_array("Y");
                    e.remove_array("B");
                    e.complex_variables.insert("Z".into(), source.clone());
                    let start = Instant::now();
                    let result = pool.install(|| e.execute(black_box(script)));
                    let elapsed = start.elapsed();
                    result.unwrap();
                    assert!(source.shares_storage(&e.complex_variables["source"]));
                    match operation {
                        "components" => {
                            assert_eq!(e.variables["R"].data()[0], source.data()[0].re);
                            assert_eq!(e.variables["I"].len(), source.len());
                        }
                        "adjoint" => {
                            let y = &e.complex_variables["Y"];
                            assert_eq!((y.rows(), y.cols()), (cols, rows));
                            assert_eq!(y.data()[0].im.to_bits(), (-source.data()[0].im).to_bits());
                        }
                        "gather" => {
                            let y = &e.complex_variables["Y"];
                            assert_eq!(y.len(), source.len().div_ceil(2));
                            assert_eq!(y.data()[y.len() - 1], source.data()[2 * (y.len() - 1)]);
                        }
                        "indexed_write" => {
                            assert_eq!(e.complex_variables["Z"].data()[0], Complex64::new(3., 4.));
                            assert_ne!(source.data()[0], Complex64::new(3., 4.));
                        }
                        _ => {
                            assert!(!e.contains_array("B"));
                            assert!(source.shares_storage(&e.complex_variables["Z"]));
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
