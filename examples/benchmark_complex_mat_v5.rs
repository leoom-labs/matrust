//! Release-only filesystem throughput; records each sample, no MATLAB speed claim.
use matrust::{Array, Complex64, ComplexMatrix, LogicalMatrix, Matrix, files};
use std::{collections::BTreeMap, hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn check(actual: &Array, expected: &Array) {
    assert_eq!(
        (actual.rows(), actual.cols()),
        (expected.rows(), expected.cols())
    );
    match (actual, expected) {
        (Array::Complex(a), Array::Complex(b)) => {
            for (x, y) in a.data().iter().zip(b.data()) {
                assert_eq!(
                    (x.re.to_bits(), x.im.to_bits()),
                    (y.re.to_bits(), y.im.to_bits())
                );
            }
        }
        (Array::Numeric(a), Array::Numeric(b)) => assert!(
            a.data()
                .iter()
                .zip(b.data())
                .all(|(x, y)| x.to_bits() == y.to_bits())
        ),
        (Array::Logical(a), Array::Logical(b)) => assert_eq!(a.data(), b.data()),
        _ => panic!("MAT storage type changed"),
    }
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(
        args.is_empty() || args == ["--reverse"],
        "Only --reverse is supported"
    );
    let reverse = !args.is_empty();
    let cancel = AtomicBool::new(false);
    let mut workers = [1, 8];
    let mut shapes = [(16, 1), (8193, 4), (262144, 4)];
    let mut kinds = ["real", "logical", "complex_zero", "complex_mixed"];
    let mut formats = [false, true];
    let mut operations = ["save", "load"];
    if reverse {
        workers.reverse();
        shapes.reverse();
        kinds.reverse();
        formats.reverse();
        operations.reverse();
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("benchmark.mat");
    println!(
        "rows,cols,kind,compressed,workers,operation,repetition,microseconds,payload_bytes,file_bytes"
    );
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for (rows, cols) in shapes {
            for kind in kinds {
                let count = rows * cols;
                let source: Array = match kind {
                    "real" => Matrix::from_column_vec(
                        rows,
                        cols,
                        (0..count).map(|i| (i % 997) as f64 / 8.).collect(),
                    )
                    .unwrap()
                    .into(),
                    "logical" => LogicalMatrix::from_column_vec(
                        rows,
                        cols,
                        (0..count).map(|i| u8::from(i % 3 == 0)).collect(),
                        &cancel,
                    )
                    .unwrap()
                    .into(),
                    _ => ComplexMatrix::from_column_vec(
                        rows,
                        cols,
                        (0..count)
                            .map(|i| {
                                Complex64::new(
                                    (i % 997) as f64 / 8.,
                                    if kind == "complex_zero" {
                                        0.
                                    } else if i % 7 == 0 {
                                        -0.
                                    } else {
                                        -(i as f64) / 16.
                                    },
                                )
                            })
                            .collect(),
                        &cancel,
                    )
                    .unwrap()
                    .into(),
                };
                let values = BTreeMap::from([("data".into(), source.clone())]);
                for compressed in formats {
                    let save = || {
                        if compressed {
                            files::save_array_workspace(&path, &values, &cancel)
                        } else {
                            files::save_array_workspace_uncompressed(&path, &values, &cancel)
                        }
                    };
                    save().unwrap();
                    for operation in operations {
                        for repetition in 0..9 {
                            // Inspection and post-result validation/destruction are not timed.
                            let catalog = files::inspect_workspace(&path, &cancel).unwrap();
                            let names = ["data".to_owned()].into_iter().collect();
                            let (elapsed, loaded) = pool.install(|| {
                                let start = Instant::now();
                                if operation == "save" {
                                    save().unwrap();
                                    (start.elapsed().as_secs_f64() * 1e6, None)
                                } else {
                                    let loaded =
                                        files::load_selected_arrays(&catalog, &names, &cancel)
                                            .unwrap();
                                    (start.elapsed().as_secs_f64() * 1e6, Some(loaded))
                                }
                            });
                            let loaded = loaded.unwrap_or_else(|| {
                                let catalog = files::inspect_workspace(&path, &cancel).unwrap();
                                files::load_selected_arrays(&catalog, &names, &cancel).unwrap()
                            });
                            check(&loaded["data"], &source);
                            black_box(&loaded);
                            assert!(values["data"].same_snapshot(&source));
                            if repetition >= 2 {
                                println!(
                                    "{rows},{cols},{kind},{compressed},{workers},{operation},{},{elapsed:.9},{},{}",
                                    repetition - 2,
                                    source.bytes(),
                                    std::fs::metadata(&path).unwrap().len()
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
