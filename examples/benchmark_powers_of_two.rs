//! Fixed public serial powers-of-two baseline; caller pools are context controls.
use matrust::{
    Matrix,
    array::{nextpow2, pow2, scale_pow2},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn patterns(operation: &str, corpus: &str) -> ([f64; 8], [f64; 8], [f64; 8]) {
    let tiny = f64::from_bits(1);
    let large = f64::from_bits(0x7fe0_0000_0000_0000);
    match (operation, corpus) {
        ("nextpow2", "ordinary") => (
            [0., -0., 0.25, -0.5, 1., 3., 8., 9.],
            [0.; 8],
            [0., 0., -2., -1., 0., 2., 3., 4.],
        ),
        ("nextpow2", "boundary") => (
            [
                tiny,
                f64::from_bits(2),
                f64::from_bits(3),
                f64::MIN_POSITIVE,
                f64::from_bits(0x000f_ffff_ffff_ffff),
                f64::MAX,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ],
            [0.; 8],
            [
                -1074.,
                -1073.,
                -1072.,
                -1022.,
                -1022.,
                1024.,
                f64::INFINITY,
                f64::INFINITY,
            ],
        ),
        ("pow2", "ordinary") => (
            [-3., -2., -1., 0., 1., 2., 3., 4.],
            [0.; 8],
            [0.125, 0.25, 0.5, 1., 2., 4., 8., 16.],
        ),
        ("pow2", "boundary") => (
            [
                -1075.,
                -1074.,
                -1022.,
                -0.,
                1023.,
                1024.,
                f64::NEG_INFINITY,
                f64::INFINITY,
            ],
            [0.; 8],
            [
                0.,
                tiny,
                f64::MIN_POSITIVE,
                1.,
                large,
                f64::INFINITY,
                0.,
                f64::INFINITY,
            ],
        ),
        ("scale_pow2", "ordinary") => (
            [1., -1., 0.5, -0.5, 3., -3., 2., -2.],
            [2.9, -2.9, 1.9, -1.9, 0.9, -0.9, 3., -3.],
            [4., -0.25, 1., -0.25, 3., -3., 16., -0.25],
        ),
        ("scale_pow2", "boundary") => (
            [
                0.5,
                tiny,
                f64::MAX,
                -tiny,
                -0.,
                f64::INFINITY,
                f64::NEG_INFINITY,
                1.,
            ],
            [
                1024.,
                1074.,
                -1023.,
                -1.,
                f64::MAX,
                -f64::MAX,
                f64::MAX,
                -1075.,
            ],
            [
                large,
                1.,
                f64::from_bits(0x3fff_ffff_ffff_ffff),
                -0.,
                -0.,
                f64::INFINITY,
                f64::NEG_INFINITY,
                0.,
            ],
        ),
        _ => unreachable!(),
    }
}

fn main() -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("benchmark_powers_of_two requires release".into());
    }
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2 || !matches!(args[1].as_str(), "normal" | "reverse") {
        return Err("benchmark_powers_of_two normal|reverse".into());
    }
    let worker_order = if args[1] == "normal" { [1, 8] } else { [8, 1] };
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    for (pool, workers) in pools.iter().zip([1, 8]) {
        pool.install(|| assert_eq!(rayon::current_num_threads(), workers));
    }
    let cancel = AtomicBool::new(false);
    println!("# 24 fixtures x 2 contexts x 9 repetitions = 432 rows/order; 336 retained, 96 warm");
    println!(
        "# complete serial public kernel + pool.install timed; construction/analytical result checks/hash/destruction excluded"
    );
    println!("order,case,operation,corpus,n,workers,rep,warmup,ns,checksum");
    let mut case = 0;
    for operation in ["nextpow2", "pow2", "scale_pow2"] {
        for corpus in ["ordinary", "boundary"] {
            let (f, e, expected) = patterns(operation, corpus);
            for n in [32usize, 4096, 65536, 1048576] {
                let input = Matrix::from_column_vec(n, 1, (0..n).map(|i| f[i % 8]).collect())?;
                let exponents = Matrix::from_column_vec(n, 1, (0..n).map(|i| e[i % 8]).collect())?;
                let aliases = (input.clone(), exponents.clone());
                let mut expected_digest = None;
                for rep in 0..9 {
                    for workers in worker_order {
                        let start = Instant::now();
                        let result =
                            black_box(pools[usize::from(workers == 8)].install(
                                || match operation {
                                    "nextpow2" => nextpow2(black_box(&input), &cancel),
                                    "pow2" => pow2(black_box(&input), &cancel),
                                    _ => scale_pow2(black_box(&input), &exponents, &cancel),
                                },
                            )?);
                        let ns = start.elapsed().as_nanos();
                        assert_eq!((result.rows(), result.cols()), (n, 1));
                        let mut hash = blake3::Hasher::new();
                        hash.update(&(n as u64).to_le_bytes());
                        hash.update(&1u64.to_le_bytes());
                        for (i, &got) in result.data().iter().enumerate() {
                            assert_eq!(
                                got.to_bits(),
                                expected[i % 8].to_bits(),
                                "{operation}/{corpus}/{i}"
                            );
                            hash.update(&got.to_bits().to_le_bytes());
                        }
                        let digest = hash.finalize().to_hex().to_string();
                        if let Some(expected) = &expected_digest {
                            assert_eq!(&digest, expected);
                        } else {
                            expected_digest = Some(digest.clone());
                        }
                        assert_eq!(input.data().as_ptr(), aliases.0.data().as_ptr());
                        assert_eq!(exponents.data().as_ptr(), aliases.1.data().as_ptr());
                        println!(
                            "{},{case},{operation},{corpus},{n},{workers},{rep},{},{ns},{digest}",
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
