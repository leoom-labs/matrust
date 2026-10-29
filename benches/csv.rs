//! Alternating, allocation-inclusive before/after CSV benchmark.
#[path = "support/csv_before.rs"]
mod before;
use matrust::files;
use std::{hint::black_box, path::Path, time::Instant};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--profile") {
        let threads = args[4].parse().unwrap();
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let start = Instant::now();
                let matrix = if args[2] == "mat" {
                    files::load_workspace(Path::new(&args[3]))
                        .map(|mut vars| vars.remove("data").unwrap())
                } else if args[2] == "before" {
                    before::read_csv(Path::new(&args[3]))
                } else {
                    files::read_csv(Path::new(&args[3]))
                }
                .unwrap();
                println!(
                    "{},{},{},{:.6}",
                    matrix.rows(),
                    matrix.cols(),
                    matrix.len(),
                    start.elapsed().as_secs_f64() * 1000.
                );
                std::thread::sleep(std::time::Duration::from_millis(250));
                black_box(matrix);
            });
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("small.csv");
    std::fs::write(&small, "1.25,2.5,-3.75,4e-3\n".repeat(256)).unwrap();
    let medium = dir.path().join("medium.csv");
    let large = dir.path().join("large.csv");
    let batch = dir.path().join("batch.csv");
    for (path, rows) in [(&medium, 16384), (&large, 65536), (&batch, 262144)] {
        std::fs::write(path, "1.25,2.5,-3.75,4e-3\n".repeat(rows)).unwrap();
    }
    println!("file,decoded_mib,threads,repetitions,before_ms,after_ms,speedup");
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                for (name, path) in [
                    ("small", small.as_path()),
                    ("medium", medium.as_path()),
                    ("large", large.as_path()),
                    ("batch", batch.as_path()),
                    (
                        "16m",
                        Path::new("target/evidence/performance/data/input_16m.csv"),
                    ),
                    (
                        "64m",
                        Path::new("target/evidence/performance/data/input_64m.csv"),
                    ),
                ] {
                    if args.iter().any(|a| a == "--sizes") && (name == "16m" || name == "64m") {
                        continue;
                    }
                    let old = before::read_csv(path).unwrap();
                    let new = files::read_csv(path).unwrap();
                    assert_eq!(old.data(), new.data());
                    assert_eq!((old.rows(), old.cols()), (new.rows(), new.cols()));
                    let mib = old.len() as f64 / 131072.;
                    drop((old, new));
                    let mut timings = [Vec::new(), Vec::new()];
                    for repetition in 0..10 {
                        for which in if repetition % 2 == 0 { [0, 1] } else { [1, 0] } {
                            let start = Instant::now();
                            let value = if which == 0 {
                                before::read_csv(path)
                            } else {
                                files::read_csv(path)
                            }
                            .unwrap();
                            black_box(&value);
                            timings[which].push(start.elapsed().as_secs_f64() * 1000.);
                            drop(value);
                        }
                    }
                    for t in &mut timings {
                        t.sort_by(f64::total_cmp);
                    }
                    let a = (timings[0][4] + timings[0][5]) * 0.5;
                    let b = (timings[1][4] + timings[1][5]) * 0.5;
                    println!("{name},{mib},{threads},10,{a:.6},{b:.6},{:.4}", a / b);
                }
            });
    }
}
