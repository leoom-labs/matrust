//! Release benchmark; one/eight worker contexts, serial HDF5 I/O, warm cache.
use matrust::files;
use std::{
    collections::BTreeSet, hint::black_box, path::Path, sync::atomic::AtomicBool, time::Instant,
};

fn main() {
    let cancel = AtomicBool::new(false);
    println!(
        "operation,format,workers,repetitions,file_bytes,selected_bytes,mean_ms,min_ms,max_ms,median_ms,samples_ms"
    );
    let mut contexts = [1, 8];
    if std::env::args().any(|arg| arg == "--reverse") {
        contexts.reverse();
    }
    for workers in contexts {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                for format in ["v73", "v73-plain"] {
                    let path = format!("target/evidence/performance/data/selective-{format}.mat");
                    let path = Path::new(&path);
                    let catalog = files::inspect_workspace(path, &cancel).unwrap();
                    assert_eq!(catalog.variables().len(), 5);
                    for name in ["small", "A"] {
                        let names = BTreeSet::from([name.to_owned()]);
                        let values =
                            files::load_selected_workspace(&catalog, &names, &cancel).unwrap();
                        let value = &values[name];
                        let rows = if name == "small" { 262144 } else { 1048576 };
                        let cols = if name == "small" { 8 } else { 16 };
                        assert_eq!((value.rows(), value.cols()), (rows, cols));
                        for (i, &v) in value.data().iter().enumerate() {
                            let expected = if name == "small" { i } else { i % rows };
                            assert_eq!(v, expected as f64 / 1024.);
                        }
                        let bytes = value.len() * 8;
                        drop(values);
                        for operation in ["inspect", "load_selected", "inspect_and_load"] {
                            let run = || match operation {
                                "inspect" => {
                                    black_box(files::inspect_workspace(path, &cancel).unwrap());
                                }
                                "load_selected" => {
                                    black_box(
                                        files::load_selected_workspace(&catalog, &names, &cancel)
                                            .unwrap(),
                                    );
                                }
                                _ => {
                                    let catalog = files::inspect_workspace(path, &cancel).unwrap();
                                    black_box(
                                        files::load_selected_workspace(&catalog, &names, &cancel)
                                            .unwrap(),
                                    );
                                }
                            };
                            run();
                            run();
                            let mut samples = Vec::new();
                            for _ in 0..10 {
                                let start = Instant::now();
                                run();
                                samples.push(start.elapsed().as_secs_f64() * 1000.);
                            }
                            let mut ordered = samples.clone();
                            ordered.sort_by(f64::total_cmp);
                            println!(
                                "{operation},{format},{workers},10,{},{bytes},{:.6},{:.6},{:.6},{:.6},\"{:?}\"",
                                catalog.file_bytes(),
                                samples.iter().sum::<f64>() / 10.,
                                samples.iter().copied().fold(f64::INFINITY, f64::min),
                                samples.iter().copied().fold(0., f64::max),
                                (ordered[4] + ordered[5]) * 0.5,
                                samples
                            );
                        }
                    }
                }
            });
    }
}
