//! Release save workflow measurements; construction and reload checks excluded.
use matrust::{Array, LogicalMatrix, Matrix, files};
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool, time::Instant};
fn verify(path: &Path, expected: &BTreeMap<String, Array>, cancel: &AtomicBool) {
    let catalog = files::inspect_workspace(path, cancel).unwrap();
    let actual =
        files::load_selected_arrays(&catalog, &expected.keys().cloned().collect(), cancel).unwrap();
    for (name, value) in expected {
        let loaded = &actual[name];
        assert_eq!(
            (value.rows(), value.cols(), value.class()),
            (loaded.rows(), loaded.cols(), loaded.class())
        );
        match (value, loaded) {
            (Array::Numeric(a), Array::Numeric(b)) => assert!(
                a.data()
                    .iter()
                    .zip(b.data())
                    .all(|(x, y)| x.to_bits() == y.to_bits())
            ),
            (Array::Logical(a), Array::Logical(b)) => assert_eq!(a.data(), b.data()),
            _ => panic!("class changed"),
        }
    }
}
fn main() {
    let cancel = AtomicBool::new(false);
    if std::env::args().any(|a| a == "--fixture") {
        let map = BTreeMap::from([
            (
                "numeric".into(),
                Matrix::from_column_vec(
                    2,
                    3,
                    vec![
                        1.,
                        -0.,
                        f64::from_bits(0x7ff8000000000042),
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        -7.25,
                    ],
                )
                .unwrap()
                .into(),
            ),
            (
                "mask".into(),
                LogicalMatrix::from_column_vec(2, 3, vec![0, 1, 1, 0, 1, 0], &cancel)
                    .unwrap()
                    .into(),
            ),
            (
                "empty_rows".into(),
                Matrix::from_column_vec(0, 7, vec![]).unwrap().into(),
            ),
            (
                "empty_columns".into(),
                Matrix::from_column_vec(5, 0, vec![]).unwrap().into(),
            ),
            (
                "empty_logical".into(),
                LogicalMatrix::filled(0, 3, false, &cancel).unwrap().into(),
            ),
            (
                "column".into(),
                Matrix::from_column_vec(5, 1, vec![1., 2., 3., 4., 5.])
                    .unwrap()
                    .into(),
            ),
            (
                "row".into(),
                Matrix::from_column_vec(1, 5, vec![1., 2., 3., 4., 5.])
                    .unwrap()
                    .into(),
            ),
            ("scalar".into(), Matrix::scalar(12.5).into()),
            (
                "chunked".into(),
                Matrix::from_column_vec(
                    5,
                    210013,
                    (0..5 * 210013).map(|i| i as f64 / 1024.).collect(),
                )
                .unwrap()
                .into(),
            ),
            (
                "long_column".into(),
                Matrix::from_column_vec(1048581, 2, (0..1048581 * 2).map(|i| i as f64).collect())
                    .unwrap()
                    .into(),
            ),
        ]);
        std::fs::create_dir_all("target/evidence/mat73-save").unwrap();
        let path = Path::new("target/evidence/mat73-save/rust-output.mat");
        files::save_array_workspace_v73(path, &map, &cancel).unwrap();
        verify(path, &map, &cancel);
        println!("Verified {} variables in {}", map.len(), path.display());
        return;
    }
    println!(
        "case,format,workers,repetitions,decoded_bytes,file_bytes,median_ms,min_ms,max_ms,samples_ms"
    );
    let mut workers = [1, 8];
    if std::env::args().any(|a| a == "--reverse") {
        workers.reverse();
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("save.mat");
    for workers in workers {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                for (case, mib) in [
                    ("ramp16", 16),
                    ("ramp128", 128),
                    ("noise16", 16),
                    ("logical16", 16),
                ] {
                    let logical = case.starts_with("logical");
                    let count = mib * 1024 * 1024 / if logical { 1 } else { 8 };
                    let array: Array = if logical {
                        LogicalMatrix::from_column_vec(
                            count / 8,
                            8,
                            (0..count).map(|i| u8::from(i % 7 == 0)).collect(),
                            &cancel,
                        )
                        .unwrap()
                        .into()
                    } else {
                        let mut state = 0x123456789abcdef0u64;
                        let data = (0..count)
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
                            .collect();
                        Matrix::from_column_vec(count / 8, 8, data).unwrap().into()
                    };
                    let bytes = array.bytes();
                    let map = BTreeMap::from([("A".into(), array)]);
                    for format in ["v7", "v73"] {
                        let run = || {
                            if format == "v73" {
                                files::save_array_workspace_v73(&path, &map, &cancel).unwrap()
                            } else {
                                files::save_array_workspace(&path, &map, &cancel).unwrap()
                            }
                        };
                        run();
                        verify(&path, &map, &cancel);
                        run();
                        let mut samples = Vec::new();
                        for _ in 0..7 {
                            let start = Instant::now();
                            run();
                            samples.push(start.elapsed().as_secs_f64() * 1000.);
                        }
                        verify(&path, &map, &cancel);
                        let mut sorted = samples.clone();
                        sorted.sort_by(f64::total_cmp);
                        println!(
                            "{case},{format},{workers},7,{bytes},{},{:.6},{:.6},{:.6},\"{:?}\"",
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
