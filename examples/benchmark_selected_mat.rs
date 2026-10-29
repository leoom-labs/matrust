use matrust::files;
use std::{
    collections::BTreeSet, hint::black_box, path::Path, sync::atomic::AtomicBool, time::Instant,
};
fn main() {
    let cancel = AtomicBool::new(false);
    let names = BTreeSet::from(["small".into()]);
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|s| s == "--profile") {
        let catalog = files::inspect_workspace(Path::new(&args[2]), &cancel).unwrap();
        let values = files::load_selected_workspace(&catalog, &names, &cancel).unwrap();
        assert_eq!(values["small"].len(), 2097152);
        println!(
            "{} values, last={}",
            values["small"].len(),
            values["small"].data()[2097151]
        );
        // Keep the loaded buffer and process alive while the parent reads the
        // OS lifetime peak counter; no sampling race at process exit.
        let mut acknowledgement = String::new();
        std::io::stdin().read_line(&mut acknowledgement).unwrap();
        black_box(values);
        return;
    }
    println!("operation,format,threads,repetitions,file_bytes,selected_bytes,mean_ms");
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                for version in [6, 7] {
                    let path = format!("target/evidence/performance/data/selective-v{version}.mat");
                    let path = Path::new(&path);
                    let catalog = files::inspect_workspace(path, &cancel).unwrap();
                    assert_eq!(catalog.variables().len(), 5);
                    let values = files::load_selected_workspace(&catalog, &names, &cancel).unwrap();
                    assert_eq!(
                        (values["small"].rows(), values["small"].cols()),
                        (262144, 8)
                    );
                    for (i, &v) in values["small"].data().iter().enumerate() {
                        assert_eq!(v, i as f64 / 1024.);
                    }
                    drop(values);
                    if version == 6 {
                        assert!(files::load_workspace(path).unwrap_err().contains("272 MiB"));
                    }
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
                        let start = Instant::now();
                        for _ in 0..10 {
                            run();
                        }
                        println!(
                            "{operation},v{version},{threads},10,{},16777216,{:.6}",
                            catalog.file_bytes(),
                            start.elapsed().as_secs_f64() * 100.
                        );
                    }
                }
            });
    }
}
