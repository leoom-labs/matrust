//! Release read API comparison. File generation, hashes, comparison and drop are untimed.
use matrust::{Array, Matrix, files};
use std::{sync::atomic::AtomicBool, time::Instant};

fn main() {
    if cfg!(debug_assertions) {
        panic!("Run release benchmarks only");
    }
    let reverse = std::env::args().any(|arg| arg == "--reverse");
    let mut workers = [1, 8];
    if reverse {
        workers.reverse();
    }
    println!(
        "rows,cols,file_bytes,workers,operation,repetitions,median_ms,min_ms,max_ms,samples_ms"
    );
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (rows, cols) in [(1024,4), (524288,4), (1,262144), (4194304,4)] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("input.csv");
                let source = Matrix::from_column_vec(rows,cols,(0..rows*cols).map(|i| (i % 1000003) as f64 / 1024. - 500.).collect()).unwrap();
                let hash = blake3::hash(bytemuck::cast_slice(source.data()));
                let cancel = AtomicBool::new(false);
                files::write_matrix_text(&path, &Array::from(source.clone()), ',', &cancel).unwrap();
                let file_bytes = std::fs::metadata(&path).unwrap().len();
                let mut operations = ["legacy_read", "cancellable_read", "strict_read"];
                if reverse { operations.reverse(); }
                let mut samples: Vec<Vec<f64>> = vec![Vec::new(); operations.len()];
                for round in 0..9 {
                    for offset in 0..operations.len() {
                        let index = (round + offset) % operations.len();
                        let start = Instant::now();
                        let loaded = match operations[index] {
                            "legacy_read" => files::read_csv(&path),
                            "cancellable_read" => files::read_csv_with_cancel(&path, &cancel),
                            "strict_read" => files::read_matrix_text(&path, None, &cancel),
                            _ => unreachable!(),
                        }.unwrap();
                        let elapsed = start.elapsed().as_secs_f64() * 1000.;
                        if round >= 2 { samples[index].push(elapsed); }
                        assert_eq!((loaded.rows(), loaded.cols()), (rows, cols));
                        assert_eq!(blake3::hash(bytemuck::cast_slice(loaded.data())), hash);
                        drop(loaded);
                    }
                }
                for (operation, samples) in operations.into_iter().zip(samples) {
                    let mut sorted = samples.clone(); sorted.sort_by(f64::total_cmp);
                    println!("{rows},{cols},{file_bytes},{workers},{operation},7,{:.6},{:.6},{:.6},\"{:?}\"",sorted[3],sorted[0],sorted[6],samples);
                }
            }
        });
    }
}
