//! Planner-inclusive FFT baseline. Existing caller-owned Rayon pools; no
//! application, MATLAB, historical or cold-cache comparison is implied.
use matrust::{
    Array, Complex64, ComplexMatrix, Matrix,
    dsp::{self, FftOptions, TransformDirection},
};
use std::{sync::atomic::AtomicBool, time::Instant};

fn components(a: &Array) -> Vec<Complex64> {
    match a {
        Array::Numeric(a) => a.data().iter().map(|&x| Complex64::new(x, 0.)).collect(),
        Array::Complex(a) => a.data().to_vec(),
        _ => panic!("Fourier result class"),
    }
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Release build required");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(
        args.is_empty() || args == ["--reverse"],
        "Only --reverse is supported"
    );
    let reverse = !args.is_empty();
    let mut cases = [
        ("small_column", 32, 1, 1, None),
        ("small_prime_row", 1, 31, 2, None),
        ("prime_columns", 257, 7, 1, None),
        ("below_parallel", 4096, 63, 1, None),
        ("parallel_columns", 4096, 64, 1, None),
        ("parallel_rows", 64, 4096, 2, None),
        ("large_prime", 65521, 1, 1, None),
        ("million_point", 1048576, 1, 1, None),
        ("padded", 1000, 8, 1, Some(1024)),
        ("truncated", 1024, 8, 1, Some(1000)),
    ];
    let mut types = ["real", "complex", "symmetric"];
    let mut directions = [TransformDirection::Forward, TransformDirection::Inverse];
    let mut workers = [1, 8];
    if reverse {
        cases.reverse();
        types.reverse();
        directions.reverse();
        workers.reverse();
    }
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    let cancel = AtomicBool::new(false);
    println!(
        "case,rows,cols,dimension,length,input_type,direction,workers,sample,output_type,output_elements,elapsed_ns"
    );
    for (case, rows, cols, dimension, length) in cases {
        let options = FftOptions {
            length,
            dimension: Some(dimension),
        };
        for kind in types {
            // For the symmetry case only DC is nonzero in each input lane, so
            // padding/truncation preserves exact conjugate symmetry.
            let input: Array = if kind == "complex" {
                ComplexMatrix::from_column_vec(
                    rows,
                    cols,
                    (0..rows * cols)
                        .map(|i| {
                            Complex64::new(
                                ((i * 17) % 127) as f64 / 64. - 1.,
                                ((i * 13) % 61) as f64 / 32. - 1.,
                            )
                        })
                        .collect(),
                    &cancel,
                )
                .unwrap()
                .into()
            } else {
                Matrix::from_column_vec(
                    rows,
                    cols,
                    (0..rows * cols)
                        .map(|i| {
                            if kind == "symmetric" {
                                if (dimension == 1 && i.is_multiple_of(rows))
                                    || (dimension == 2 && i < rows)
                                {
                                    2.
                                } else {
                                    0.
                                }
                            } else {
                                ((i * 17) % 127) as f64 / 64. - 1.
                            }
                        })
                        .collect(),
                )
                .unwrap()
                .into()
            };
            let alias = input.clone();
            for direction in directions {
                let expected = pools[0]
                    .install(|| {
                        dsp::fourier(&input, options, direction, 256 * 1024 * 1024, &cancel)
                    })
                    .unwrap();
                let expected_values = components(&expected);
                let (out_rows, out_cols) = dsp::fourier_shape(&input, options).unwrap();
                let n = if dimension == 1 { out_rows } else { out_cols };
                if kind == "symmetric" {
                    let expected_scalar = if direction == TransformDirection::Inverse {
                        2. / n as f64
                    } else {
                        2.
                    };
                    assert!(
                        expected_values
                            .iter()
                            .all(|z| (z.re - expected_scalar).abs() < 2e-12 && z.im.abs() < 2e-12)
                    );
                    if direction == TransformDirection::Inverse {
                        assert!(matches!(&expected, Array::Numeric(_)));
                    }
                }
                for worker in workers {
                    let pool = &pools[usize::from(worker == 8)];
                    for repetition in 0..9 {
                        let start = Instant::now();
                        let output = pool
                            .install(|| {
                                dsp::fourier(&input, options, direction, 256 * 1024 * 1024, &cancel)
                            })
                            .unwrap();
                        let elapsed = start.elapsed().as_nanos();
                        assert_eq!((output.rows(), output.cols()), (out_rows, out_cols));
                        assert_eq!(output.is_complex(), expected.is_complex());
                        assert!(
                            components(&output)
                                .iter()
                                .zip(&expected_values)
                                .all(|(a, b)| a.re.to_bits() == b.re.to_bits()
                                    && a.im.to_bits() == b.im.to_bits())
                        );
                        assert!(input.same_snapshot(&alias));
                        if repetition >= 2 {
                            let direction = if direction == TransformDirection::Inverse {
                                "ifft"
                            } else {
                                "fft"
                            };
                            let class = if output.is_complex() {
                                "complex"
                            } else {
                                "double"
                            };
                            println!(
                                "{case},{rows},{cols},{dimension},{n},{kind},{direction},{worker},{},{class},{},{elapsed}",
                                repetition - 2,
                                out_rows * out_cols
                            );
                        }
                        drop(output); // Explicitly outside elapsed time.
                    }
                }
            }
        }
    }
}
