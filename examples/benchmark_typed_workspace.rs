use matrust::{Array, LogicalMatrix, Matrix, files};
use std::{
    collections::{BTreeMap, BTreeSet},
    hint::black_box,
    sync::atomic::AtomicBool,
    time::Instant,
};
fn main() {
    let threads: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&threads));
    println!("case,elements,threads,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(|| {
            let cancel = AtomicBool::new(false);
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("workspace.mat");
            let csv = dir.path().join("values.csv");
            for n in [65536, 1048576, 8388608] {
                let m = Matrix::from_column_vec(
                    n / 8,
                    8,
                    (0..n).map(|i| (i % 1000) as f64 / 8.).collect(),
                )
                .unwrap();
                let mask = LogicalMatrix::from_column_vec(
                    n / 8,
                    8,
                    (0..n).map(|i| u8::from(i % 3 == 0)).collect(),
                    &cancel,
                )
                .unwrap();
                let numeric = BTreeMap::from([("A".into(), m.clone())]);
                let typed = BTreeMap::from([("A".into(), Array::Numeric(m.clone()))]);
                let mixed = BTreeMap::from([
                    ("A".into(), Array::Numeric(m.clone())),
                    ("mask".into(), Array::Logical(mask.clone())),
                ]);
                files::save_workspace(&path, &numeric).unwrap();
                let legacy_bytes = std::fs::metadata(&path).unwrap().len();
                files::save_array_workspace(&path, &typed, &cancel).unwrap();
                let typed_bytes = std::fs::metadata(&path).unwrap().len();
                eprintln!("elements={n}, legacy_compressed_bytes={legacy_bytes}, typed_compressed_bytes={typed_bytes}");
                files::save_array_workspace(&path, &mixed, &cancel).unwrap();
                let catalog = files::inspect_workspace(&path, &cancel).unwrap();
                let names = BTreeSet::from(["A".into(), "mask".into()]);
                let restored = files::load_selected_arrays(&catalog, &names, &cancel).unwrap();
                assert_eq!(restored["A"].as_numeric().unwrap().data(), m.data());
                let Array::Logical(result) = &restored["mask"] else {
                    panic!("lost logical type")
                };
                assert_eq!(result.data(), mask.data());
                drop(restored);
                let repeats = 11;
                let mut old = Vec::new();
                let mut new = Vec::new();
                for sample in 0..repeats + 3 {
                    for variant in if sample % 2 == 0 { [0, 1] } else { [1, 0] } {
                        let start = Instant::now();
                        if variant == 0 {
                            files::save_workspace(&path, black_box(&numeric)).unwrap();
                        } else {
                            files::save_array_workspace(&path, black_box(&typed), &cancel).unwrap();
                        }
                        if sample >= 3 {
                            if variant == 0 { &mut old } else { &mut new }
                                .push(start.elapsed().as_secs_f64() * 1000.);
                        }
                    }
                }
                let print = |name: &str, mut times: Vec<f64>| {
                    times.sort_by(f64::total_cmp);
                    println!(
                        "{name},{n},{threads},{repeats},{:.6},{:.6},{:.6}",
                        times[5], times[0], times[10]
                    );
                };
                print("numeric_save_legacy", old);
                print("numeric_save_typed", new);
                let decoded = files::load_workspace(&path).unwrap();
                assert_eq!(decoded["A"].data(), m.data());
                drop(decoded);
                files::save_array_workspace(&path, &mixed, &cancel).unwrap();
                let catalog = files::inspect_workspace(&path, &cancel).unwrap();
                let run = |name: &str, f: &mut dyn FnMut()| {
                    for _ in 0..3 {
                        f();
                    }
                    let mut times = Vec::new();
                    for _ in 0..repeats {
                        let start = Instant::now();
                        f();
                        times.push(start.elapsed().as_secs_f64() * 1000.);
                    }
                    print(name, times);
                };
                run("mixed_load_typed", &mut || {
                    black_box(files::load_selected_arrays(&catalog, &names, &cancel).unwrap());
                });
                run("mixed_save_typed", &mut || {
                    files::save_array_workspace(&path, black_box(&mixed), &cancel).unwrap();
                });
                run("mixed_save_uncompressed", &mut || {
                    files::save_array_workspace_uncompressed(&path, black_box(&mixed), &cancel)
                        .unwrap();
                });
                let catalog = files::inspect_workspace(&path, &cancel).unwrap();
                run("mixed_load_uncompressed", &mut || {
                    black_box(files::load_selected_arrays(&catalog, &names, &cancel).unwrap());
                });
                // CSV formatting is sequential; keep this comparison to 8 MiB input.
                if n <= 1048576 {
                    files::write_csv(&csv, &m).unwrap();
                    let expected = std::fs::read(&csv).unwrap();
                    let mut old = Vec::new();
                    let mut new = Vec::new();
                    for sample in 0..repeats + 3 {
                        for variant in if sample % 2 == 0 { [0, 1] } else { [1, 0] } {
                            let start = Instant::now();
                            if variant == 0 {
                                files::write_csv(&csv, black_box(&m)).unwrap();
                            } else {
                                files::write_array_csv(&csv, black_box(&typed["A"]), &cancel).unwrap();
                            }
                            if sample >= 3 {
                                if variant == 0 { &mut old } else { &mut new }
                                    .push(start.elapsed().as_secs_f64() * 1000.);
                            }
                        }
                    }
                    print("numeric_csv_legacy", old);
                    print("numeric_csv_typed", new);
                    assert_eq!(std::fs::read(&csv).unwrap(), expected);
                }
            }
        });
}
