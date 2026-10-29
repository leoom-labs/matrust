//! Compare the same eager scalar kernels with/without bounded coordinate facts.
use matrust::Matrix;
use std::{hint::black_box, time::Instant};

fn main() {
    if cfg!(debug_assertions) {
        panic!("Use a release artifact");
    }
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(args.is_empty() || args == ["--reverse"]);
    let mut schedule = Vec::new();
    for elements in [1, 256, 65_536, 1_000_000, 1_048_576] {
        for workers in [1, 8] {
            for operation in ["propagate", "dense_zip_control"] {
                schedule.push((elements, workers, operation));
            }
        }
    }
    if !args.is_empty() {
        schedule.reverse();
    }
    println!(
        "elements\tworkers\toperation\tsample\tnanoseconds\tlogical_bytes\tretained_bytes\tproof_disposition"
    );
    for (elements, workers, operation) in schedule {
        let source = Matrix::from_column_vec(
            elements,
            1,
            (0..elements)
                .map(|i| if i == 0 { -0.0 } else { i as f64 * 0.125 })
                .collect(),
        )
        .unwrap();
        let alias = source.clone();
        let offset = Matrix::scalar(0.25);
        let half = Matrix::scalar(0.4);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for repetition in 0..9 {
            let start = Instant::now();
            let output = pool.install(|| match operation {
                "propagate" => source
                    .coordinate_add(&offset)
                    .unwrap()
                    .coordinate_subtract(&half)
                    .unwrap(),
                "dense_zip_control" => source
                    .zip(&offset, |a, b| a + b)
                    .unwrap()
                    .zip(&half, |a, b| a - b)
                    .unwrap(),
                _ => unreachable!(),
            });
            let elapsed = start.elapsed().as_nanos();
            black_box(&output);
            assert_eq!((output.rows(), output.cols()), (elements, 1));
            for (&a, &input) in output.data().iter().zip(source.data()) {
                assert_eq!(a.to_bits(), ((input + 0.25) - 0.4).to_bits());
            }
            assert_eq!(source.data().as_ptr(), alias.data().as_ptr());
            let expected_proof = operation == "propagate" && elements <= 1_000_000;
            assert_eq!(output.has_coordinate_proof(), expected_proof);
            let disposition = if expected_proof {
                "retained"
            } else if operation == "propagate" {
                "limit_fallback"
            } else {
                "control"
            };
            if repetition >= 2 {
                println!(
                    "{elements}\t{workers}\t{operation}\t{}\t{elapsed}\t{}\t{}\t{disposition}",
                    repetition - 2,
                    output.len() * 8,
                    output.retained_bytes()
                );
            }
            drop(output);
        }
    }
}
