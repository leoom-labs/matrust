//! Release kernel allocation/computation; no MATLAB or historical speed claim.
use matrust::{Matrix, dsp};
use std::{sync::atomic::AtomicBool, time::Instant};

fn run(kind: &str, x: &Matrix, h: &Matrix, cancel: &AtomicBool) -> Matrix {
    match kind {
        "upsample" => dsp::upsample(x, 3, 1, cancel).unwrap(),
        "downsample" => dsp::downsample(x, 2, 1, cancel).unwrap(),
        _ => dsp::upfirdn(x, h, 3, 2, cancel).unwrap(),
    }
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Release build required");
    }
    let reverse = std::env::args().any(|a| a == "--reverse");
    let mut workers = [1, 8];
    let mut shapes = [
        (128, 1),
        (65536, 1),
        (65536, 8),
        (131070, 8),
        (131072, 8),
        (8, 65536),
    ];
    if reverse {
        workers.reverse();
        shapes.reverse();
    }
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    println!("rows,cols,kind,workers,repetitions,median_us,min_us,max_us,samples_us");
    for (rows, cols) in shapes {
        let x = Matrix::from_column_vec(
            rows,
            cols,
            (0..rows * cols)
                .map(|i| ((i * 173) % 65521) as f64 / 65536. - 0.5)
                .collect(),
        )
        .unwrap();
        let h = Matrix::from_column_vec(33, 1, (0..33).map(|i| (i as f64 - 16.) / 1024.).collect())
            .unwrap();
        let mut kinds = ["upsample", "downsample", "upfirdn"];
        if reverse {
            kinds.reverse();
        }
        for kind in kinds {
            let cancel = AtomicBool::new(false);
            let expected = pools[0].install(|| run(kind, &x, &h, &cancel));
            for workers in workers {
                let pool = &pools[usize::from(workers == 8)];
                let mut samples = Vec::new();
                for repetition in 0..9 {
                    let begin = Instant::now();
                    let output = pool.install(|| run(kind, &x, &h, &cancel));
                    let elapsed = begin.elapsed().as_secs_f64() * 1e6;
                    if repetition >= 2 {
                        samples.push(elapsed);
                    }
                    assert_eq!(
                        (output.rows(), output.cols()),
                        (expected.rows(), expected.cols())
                    );
                    assert!(
                        output
                            .data()
                            .iter()
                            .zip(expected.data())
                            .all(|(a, b)| a.to_bits() == b.to_bits())
                    );
                    drop(output);
                }
                let mut ordered = samples.clone();
                ordered.sort_by(f64::total_cmp);
                println!(
                    "{rows},{cols},{kind},{workers},7,{:.6},{:.6},{:.6},\"{:?}\"",
                    ordered[3], ordered[0], ordered[6], samples
                );
            }
        }
    }
}
