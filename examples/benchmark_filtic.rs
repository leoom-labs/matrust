//! Fixed public serial filtic baseline; caller pools are context controls only.
use matrust::{Matrix, dsp};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn main() -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("benchmark_filtic requires release".into());
    }
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2 || !matches!(args[1].as_str(), "normal" | "reverse") {
        return Err("benchmark_filtic normal|reverse".into());
    }
    let worker_order = if args[1] == "normal" { [1, 8] } else { [8, 1] };
    let pools = [1, 8].map(|count| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(count)
            .build()
            .unwrap()
    });
    for (pool, workers) in pools.iter().zip([1, 8]) {
        pool.install(|| assert_eq!(rayon::current_num_threads(), workers));
    }
    let cancel = AtomicBool::new(false);
    println!(
        "# 56 fixtures x 2 contexts x 9 repetitions = 1008 rows/order; 784 retained, 224 warm"
    );
    println!(
        "# complete serial public kernel + pool.install timed; fixture construction/checks/hash/destruction excluded"
    );
    println!("order,case,states,iir,history,workers,rep,warmup,ns,checksum");
    let mut case = 0;
    for order in [0usize, 1, 2, 16, 256, 1024, 4096] {
        for iir in [false, true] {
            for history in ["empty", "short", "full", "overlong"] {
                let b = Matrix::from_column_vec(1, order + 1, vec![1.; order + 1])?;
                let mut a = vec![-0.5; if iir { order + 1 } else { 1 }];
                a[0] = 1.;
                let a = Matrix::from_column_vec(1, a.len(), a)?;
                let length = match history {
                    "empty" => 0,
                    "short" => order.min(3),
                    "full" => order,
                    _ => order + 65536,
                };
                let values = (0..length)
                    .map(|i| if i < order { 1. } else { f64::NAN })
                    .collect();
                let x = Matrix::from_column_vec(1, length, values)?;
                let y = x.clone();
                let aliases = (b.clone(), a.clone(), x.clone(), y.clone());
                let mut expected_hash = None;
                for rep in 0..9 {
                    for workers in worker_order {
                        let start = Instant::now();
                        let result = black_box(pools[usize::from(workers == 8)].install(|| {
                            dsp::filtic(
                                black_box(b.data()),
                                a.data(),
                                y.data(),
                                Some(x.data()),
                                &cancel,
                            )
                        })?);
                        let ns = start.elapsed().as_nanos();
                        assert_eq!((result.rows(), result.cols()), (order, 1));
                        let mut hash = blake3::Hasher::new();
                        hash.update(&(order as u64).to_le_bytes());
                        hash.update(&1u64.to_le_bytes());
                        for (k, &got) in result.data().iter().enumerate() {
                            // Unit histories and constant tails give an exact
                            // count, independently of the implementation's sums.
                            let count = (order - k).min(length) as f64;
                            let expected = if iir { 1.5 * count } else { count };
                            assert_eq!(got.to_bits(), expected.to_bits());
                            hash.update(&got.to_bits().to_le_bytes());
                        }
                        let digest = hash.finalize().to_hex().to_string();
                        if let Some(expected) = &expected_hash {
                            assert_eq!(&digest, expected);
                        } else {
                            expected_hash = Some(digest.clone());
                        }
                        for (value, alias) in [
                            (&b, &aliases.0),
                            (&a, &aliases.1),
                            (&x, &aliases.2),
                            (&y, &aliases.3),
                        ] {
                            assert_eq!(value.data().as_ptr(), alias.data().as_ptr());
                        }
                        println!(
                            "{},{case},{order},{iir},{history},{workers},{rep},{},{ns},{digest}",
                            args[1],
                            usize::from(rep < 2)
                        );
                    }
                }
                case += 1;
            }
        }
    }
    assert_eq!(case, 56);
    Ok(())
}
