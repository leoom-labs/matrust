//! Release-only serial HDF5 I/O under one/eight configured workers; no scaling claim.
use matrust::{Array, Complex64, ComplexMatrix, files};
use std::{
    collections::{BTreeMap, BTreeSet},
    hint::black_box,
    sync::atomic::AtomicBool,
    time::Instant,
};
fn main() {
    if cfg!(debug_assertions) {
        panic!("Run this benchmark in release mode");
    }
    let reverse = std::env::args().any(|a| a == "--reverse");
    let mut groups = Vec::new();
    for workers in [1, 8] {
        for (rows, cols) in [
            (1, 1),
            (16, 15),
            (16, 16),
            (16, 17),
            (524291, 1),
            (3, 174765),
            (1024, 8192),
        ] {
            for kind in ["read", "write"] {
                groups.push((workers, rows, cols, kind));
            }
        }
    }
    if reverse {
        groups.reverse();
    }
    println!("workers,rows,cols,operation,sample,microseconds");
    for (workers, rows, cols, kind) in groups {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let stop = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("complex.mat");
        let source: Array = ComplexMatrix::from_column_vec(
            rows,
            cols,
            (0..rows * cols)
                .map(|i| Complex64::new((i % 997) as f64 / 128., -((i % 991) as f64) / 256.))
                .collect(),
            &stop,
        )
        .unwrap()
        .into();
        let vars = BTreeMap::from([("Z".into(), source.clone())]);
        let selected = BTreeSet::from(["Z".into()]);
        files::save_array_workspace_v73(&path, &vars, &stop).unwrap();
        let mut samples = Vec::new();
        for sample in 0..9 {
            let elapsed = pool.install(|| {
                let started = Instant::now();
                if kind == "read" {
                    let cat = files::inspect_workspace(&path, &stop).unwrap();
                    let output = files::load_selected_arrays(&cat, &selected, &stop).unwrap();
                    let elapsed = started.elapsed();
                    assert_eq!(
                        output["Z"].as_complex().unwrap().data(),
                        source.as_complex().unwrap().data()
                    );
                    black_box(output);
                    elapsed
                } else {
                    files::save_array_workspace_v73(&path, &vars, &stop).unwrap();
                    started.elapsed()
                }
            });
            if sample >= 2 {
                samples.push(elapsed.as_secs_f64() * 1e6);
            }
        }
        let cat = files::inspect_workspace(&path, &stop).unwrap();
        let check = files::load_selected_arrays(&cat, &selected, &stop).unwrap();
        assert_eq!(
            check["Z"].as_complex().unwrap().data(),
            source.as_complex().unwrap().data()
        );
        assert!(source.same_snapshot(&vars["Z"]));
        for (sample, microseconds) in samples.iter().enumerate() {
            println!("{workers},{rows},{cols},{kind},{sample},{microseconds:.6}");
        }
    }
}
