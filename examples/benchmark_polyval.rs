//! Fixed public polynomial release measurements with bounded production dispatch.
use matrust::{Matrix, array::evaluate_polynomial};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn main() -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("benchmark_polyval requires release".into());
    }
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2 || !matches!(args[1].as_str(), "normal" | "reverse") {
        return Err("benchmark_polyval normal|reverse".into());
    }
    let workers = if args[1] == "normal" { [1, 8] } else { [8, 1] };
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    for (pool, count) in pools.iter().zip([1, 8]) {
        pool.install(|| assert_eq!(rayon::current_num_threads(), count));
    }
    let cancel = AtomicBool::new(false);
    println!("# 24 cases x 2 contexts x 9 repetitions = 432 rows/order; 336 retained, 96 warm");
    println!(
        "# complete public kernel + pool.install timed; construction/validation/hash/destruction excluded"
    );
    println!("order,case,coefficients,queries,matrix,workers,rep,warmup,ns,checksum");
    let mut case = 0;
    for coefficients in [1usize, 4, 17] {
        for n in [32usize, 4096, 262144, 1048576] {
            for matrix in [false, true] {
                let p = match coefficients {
                    1 => vec![-0.],
                    4 => vec![2., -3., 0., 5.],
                    _ => {
                        let mut p = vec![0.; 17];
                        p[0] = 1.;
                        p
                    }
                };
                let p = Matrix::from_column_vec(1, coefficients, p)?;
                let x = Matrix::from_column_vec(
                    if matrix { 2 } else { 1 },
                    if matrix { n / 2 } else { n },
                    (0..n).map(|i| (i % 5) as f64 / 2. - 1.).collect(),
                )?;
                let aliases = (p.clone(), x.clone());
                let mut expected_hash = None;
                for rep in 0..9 {
                    for workers in workers {
                        let start = Instant::now();
                        let result = black_box(
                            pools[usize::from(workers == 8)]
                                .install(|| evaluate_polynomial(black_box(&p), &x, &cancel))?,
                        );
                        let ns = start.elapsed().as_nanos();
                        assert_eq!((result.rows(), result.cols()), (x.rows(), x.cols()));
                        let mut hash = blake3::Hasher::new();
                        hash.update(&(x.rows() as u64).to_le_bytes());
                        hash.update(&(x.cols() as u64).to_le_bytes());
                        for (&query, &got) in x.data().iter().zip(result.data()) {
                            let expected = match coefficients {
                                1 => -0.,
                                4 => 2. * query * query * query - 3. * query * query + 5.,
                                _ if query.abs() == 1. => 1.,
                                _ if query == 0. => 0.,
                                _ => 1. / 65536.,
                            };
                            assert_eq!(got.to_bits(), expected.to_bits());
                            hash.update(&got.to_bits().to_le_bytes());
                        }
                        let digest = hash.finalize().to_hex().to_string();
                        if let Some(expected) = &expected_hash {
                            assert_eq!(&digest, expected);
                        } else {
                            expected_hash = Some(digest.clone());
                        }
                        assert_eq!(p.data().as_ptr(), aliases.0.data().as_ptr());
                        assert_eq!(x.data().as_ptr(), aliases.1.data().as_ptr());
                        println!(
                            "{},{case},{coefficients},{n},{matrix},{workers},{rep},{},{ns},{digest}",
                            args[1],
                            usize::from(rep < 2)
                        );
                    }
                }
                case += 1;
            }
        }
    }
    assert_eq!(case, 24);
    Ok(())
}
