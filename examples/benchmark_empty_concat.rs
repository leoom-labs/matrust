//! Focused release probe for 0x0 concatenation operands; no speedup claim.
use matrust::Matrix;
use std::{hint::black_box, time::Instant};

fn main() {
    println!("workers,rows,cols,repetitions,axis,empty_operand,median_us");
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (rows, cols, repetitions) in [(1, 3, 10000), (1, 1024, 1000), (1024, 1024, 20)] {
                let a = Matrix::filled(rows, cols, 1.).unwrap();
                let empty = Matrix::filled(0, 0, 0.).unwrap();
                for vertical in [false, true] {
                    for include_empty in [false, true] {
                        let concatenate = || {
                            let blocks = match (vertical, include_empty) {
                                (false, false) => vec![vec![a.clone(), a.clone()]],
                                (false, true) => vec![vec![a.clone(), empty.clone(), a.clone()]],
                                (true, false) => vec![vec![a.clone()], vec![a.clone()]],
                                (true, true) => {
                                    vec![vec![a.clone()], vec![empty.clone()], vec![a.clone()]]
                                }
                            };
                            Matrix::concatenate(blocks).unwrap()
                        };
                        let result = concatenate();
                        assert_eq!(result.rows(), rows * if vertical { 2 } else { 1 });
                        assert_eq!(result.cols(), cols * if vertical { 1 } else { 2 });
                        assert!(result.data().iter().all(|v| *v == 1.));
                        for _ in 0..5 {
                            black_box(concatenate());
                        }
                        let mut samples = [0.; 5];
                        for sample in &mut samples {
                            let start = Instant::now();
                            for _ in 0..repetitions {
                                black_box(concatenate());
                            }
                            *sample = start.elapsed().as_secs_f64() * 1e6 / repetitions as f64;
                        }
                        samples.sort_by(f64::total_cmp);
                        println!(
                            "{workers},{rows},{cols},{repetitions},{},{include_empty},{:.3}",
                            if vertical { "vertical" } else { "horizontal" },
                            samples[2]
                        );
                    }
                }
            }
        });
    }
}
