//! Compare the same bounded file workflow through the API and script front end.
use matrust::{Array, Engine, Matrix, files};
use std::{collections::BTreeMap, sync::atomic::AtomicBool, time::Instant};

fn main() {
    let mut workers = [1, 8];
    if std::env::args().any(|s| s == "--reverse") {
        workers.reverse();
    }
    println!(
        "case,operation,workers,repetitions,decoded_bytes,file_bytes,median_ms,min_ms,max_ms,samples_ms"
    );
    for workers in workers {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                for (case, mib) in [("ramp16", 16), ("ramp128", 128), ("noise16", 16)] {
                    let count = mib * 1024 * 1024 / 8;
                    let mut state = 0x123456789abcdef0u64;
                    let matrix = Matrix::from_column_vec(
                        count / 8,
                        8,
                        (0..count)
                            .map(|i| {
                                if case.starts_with("noise") {
                                    state ^= state << 13;
                                    state ^= state >> 7;
                                    state ^= state << 17;
                                    (state >> 11) as f64 / 9007199254740992.
                                } else {
                                    i as f64 / 1024.
                                }
                            })
                            .collect(),
                    )
                    .unwrap();
                    let expected = BTreeMap::from([("A".to_owned(), Array::from(matrix.clone()))]);
                    let selected = expected.keys().cloned().collect();
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("data.mat");
                    let cancel = AtomicBool::new(false);
                    let mut engine = Engine::default();
                    engine.set_file_directory(dir.path()).unwrap();
                    engine.variables.insert("A".into(), matrix.clone());
                    let mut operations = [
                        "direct_save",
                        "function_save",
                        "command_save",
                        "direct_load",
                        "function_load",
                        "command_load",
                    ];
                    // Rotate operations each round; reverse their order on the second run.
                    if std::env::args().any(|s| s == "--reverse") {
                        operations.reverse();
                    }
                    files::save_array_workspace_v73(&path, &expected, &cancel).unwrap();
                    let mut samples = vec![Vec::new(); operations.len()];
                    for round in 0..9 {
                        for offset in 0..operations.len() {
                            let index = (offset + round) % operations.len();
                            let operation = operations[index];
                            let start = Instant::now();
                            match operation {
                                "direct_save" => files::save_array_workspace_v73(
                                    &path,
                                    &engine.workspace(),
                                    &cancel,
                                )
                                .unwrap(),
                                "function_save" => {
                                    engine.execute("save('data.mat','A','-v7.3');").unwrap();
                                }
                                "command_save" => {
                                    engine.execute("save data.mat A -v7.3;").unwrap();
                                }
                                "direct_load" => {
                                    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
                                    engine
                                        .merge_workspace(
                                            files::load_selected_arrays(
                                                &catalog, &selected, &cancel,
                                            )
                                            .unwrap(),
                                        )
                                        .unwrap();
                                }
                                "function_load" => {
                                    engine.execute("load('data.mat','A');").unwrap();
                                }
                                "command_load" => {
                                    engine.execute("load data.mat A;").unwrap();
                                }
                                _ => unreachable!(),
                            };
                            let elapsed = start.elapsed().as_secs_f64() * 1000.;
                            if round >= 2 {
                                samples[index].push(elapsed);
                            }
                        }
                    }
                    for (operation, samples) in operations.into_iter().zip(samples) {
                        let catalog = files::inspect_workspace(&path, &cancel).unwrap();
                        let loaded =
                            files::load_selected_arrays(&catalog, &selected, &cancel).unwrap();
                        for actual in [&loaded["A"], &engine.array("A").unwrap()] {
                            let Array::Numeric(actual) = actual else {
                                panic!("class changed")
                            };
                            assert_eq!(
                                (actual.rows(), actual.cols()),
                                (matrix.rows(), matrix.cols())
                            );
                            assert!(
                                actual
                                    .data()
                                    .iter()
                                    .zip(matrix.data())
                                    .all(|(a, b)| a.to_bits() == b.to_bits())
                            );
                        }
                        let mut sorted = samples.clone();
                        sorted.sort_by(f64::total_cmp);
                        println!(
                            "{case},{operation},{workers},7,{},{},{:.6},{:.6},{:.6},\"{:?}\"",
                            count * 8,
                            path.metadata().unwrap().len(),
                            sorted[3],
                            sorted[0],
                            sorted[6],
                            samples
                        );
                    }
                }
            });
    }
}
