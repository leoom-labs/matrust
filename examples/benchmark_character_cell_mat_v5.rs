//! Serial MAT codecs; pool entry is included and one/eight are configurations.
use matrust::{Array, CellArray, CharacterRow, Complex64, ComplexMatrix, Matrix, files};
use std::{
    collections::BTreeMap,
    hint::black_box,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
fn verify(a: &Array, b: &Array) {
    assert_eq!(
        (a.class(), a.rows(), a.cols(), a.is_complex()),
        (b.class(), b.rows(), b.cols(), b.is_complex())
    );
    match (a, b) {
        (Array::Character(a), Array::Character(b)) => assert_eq!(a.units(), b.units()),
        (Array::Cell(a), Array::Cell(b)) => {
            for i in 0..a.len() {
                verify(a.get_linear(i).unwrap(), b.get_linear(i).unwrap());
            }
        }
        (Array::Numeric(a), Array::Numeric(b)) => {
            for (a, b) in a.data().iter().zip(b.data()) {
                assert_eq!(a.to_bits(), b.to_bits());
            }
        }
        (Array::Logical(a), Array::Logical(b)) => assert_eq!(a.data(), b.data()),
        (Array::Complex(a), Array::Complex(b)) => {
            for (a, b) in a.data().iter().zip(b.data()) {
                assert_eq!(
                    (a.re.to_bits(), a.im.to_bits()),
                    (b.re.to_bits(), b.im.to_bits())
                );
            }
        }
        _ => panic!("Unexpected typed array"),
    }
}
fn fixture(name: &str, stop: &AtomicBool) -> Array {
    match name {
        "text1" => CharacterRow::from_text("A", stop).unwrap().into(),
        "text65536" => CharacterRow::from_text(&"\u{03bb}".repeat(65536), stop)
            .unwrap()
            .into(),
        "cells16x16" => {
            let value: Array = Matrix::filled(1, 128, 3.).unwrap().into();
            let shared = Arc::new(value);
            CellArray::from_shared(16, 16, vec![shared; 256], stop)
                .unwrap()
                .into()
        }
        "cells8193" => CellArray::empty(1, 8193, stop)
            .unwrap()
            .replaced(0, Matrix::filled(1, 1048576, 2.).unwrap().into(), stop)
            .unwrap()
            .into(),
        "nested16" => {
            let mut value: Array = ComplexMatrix::filled(1, 4096, Complex64::new(-0., 3.), stop)
                .unwrap()
                .into();
            for _ in 0..16 {
                value = CellArray::from_shared(1, 1, vec![Arc::new(value)], stop)
                    .unwrap()
                    .into();
            }
            value
        }
        _ => panic!("Unknown fixture"),
    }
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(args.is_empty() || args == ["--reverse"]);
    let mut cases = vec!["text1", "text65536", "cells16x16", "cells8193", "nested16"];
    let mut workers = vec![1, 8];
    let mut jobs = vec![
        "save_plain",
        "save_compressed",
        "load_plain",
        "load_compressed",
        "inspect_plain",
        "inspect_compressed",
    ];
    if !args.is_empty() {
        cases.reverse();
        workers.reverse();
        jobs.reverse();
    }
    println!("workers,case,operation,rows,cols,logical_bytes,encoded_bytes,sample,nanoseconds");
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for case in &cases {
            let stop = AtomicBool::new(false);
            let value = fixture(case, &stop);
            let alias = value.clone();
            let values = BTreeMap::from([("value".into(), value.clone())]);
            let dir = tempfile::tempdir().unwrap();
            let plain = dir.path().join("plain.mat");
            let compressed = dir.path().join("compressed.mat");
            files::save_array_workspace_uncompressed(&plain, &values, &stop).unwrap();
            files::save_array_workspace(&compressed, &values, &stop).unwrap();
            for &job in &jobs {
                let path = if job.ends_with("compressed") {
                    &compressed
                } else {
                    &plain
                };
                // A retained catalog is required only for selected-load timing.
                let catalog = files::inspect_workspace(path, &stop).unwrap();
                let names = values.keys().cloned().collect();
                for iteration in 0..9 {
                    let start = Instant::now();
                    let output = pool.install(|| match job {
                        "save_plain" => {
                            files::save_array_workspace_uncompressed(
                                black_box(path),
                                &values,
                                &stop,
                            )
                            .unwrap();
                            None
                        }
                        "save_compressed" => {
                            files::save_array_workspace(black_box(path), &values, &stop).unwrap();
                            None
                        }
                        "load_plain" | "load_compressed" => Some(
                            files::load_selected_arrays(black_box(&catalog), &names, &stop)
                                .unwrap(),
                        ),
                        _ => {
                            let result = files::inspect_workspace(black_box(path), &stop).unwrap();
                            black_box(result);
                            None
                        }
                    });
                    let elapsed = start.elapsed();
                    if let Some(loaded) = output {
                        verify(&value, &loaded["value"]);
                    } else if job.starts_with("save") {
                        let current = files::inspect_workspace(path, &stop).unwrap();
                        let loaded = files::load_selected_arrays(&current, &names, &stop).unwrap();
                        verify(&value, &loaded["value"]);
                    }
                    assert!(value.same_snapshot(&alias));
                    let encoded = std::fs::metadata(path).unwrap().len();
                    if iteration >= 2 {
                        println!(
                            "{workers},{case},{job},{},{},{},{encoded},{},{}",
                            value.rows(),
                            value.cols(),
                            value.bytes(),
                            iteration - 2,
                            elapsed.as_nanos()
                        );
                    }
                }
            }
        }
    }
}
