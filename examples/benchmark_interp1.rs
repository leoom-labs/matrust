//! Fixed release-only complete interpolation kernel comparison. Root owns execution.
use matrust::{
    Matrix,
    array::{InterpolationMethod, interpolate_vectors},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("benchmark_interp1 requires release".into());
    }
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2 || !matches!(args[1].as_str(), "normal" | "reverse") {
        return Err("benchmark_interp1 normal|reverse".into());
    }
    let workers = if args[1] == "normal" { [1, 8] } else { [8, 1] };
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    for (slot, pool) in pools.iter().enumerate() {
        pool.install(|| assert_eq!(rayon::current_num_threads(), if slot == 0 { 1 } else { 8 }));
    }
    let cancel = AtomicBool::new(false);
    println!(
        "# 64 cases x 2 caller contexts x 9 repetitions: 1152 rows/order, 896 retained, 256 warm"
    );
    println!(
        "# complete kernel, query execution serial; existing sort may use caller pool; no MATLAB/before-after claim"
    );
    println!("order,case,n,m,method,unordered,workers,rep,warmup,ns,checksum");
    let mut case = 0;
    for n in [2usize, 32, 4096, 65536] {
        for m in [32usize, 4096, 262144, 1048576] {
            for method in [InterpolationMethod::Linear, InterpolationMethod::Nearest] {
                for unordered in [false, true] {
                    let x = Matrix::from_column_vec(
                        1,
                        n,
                        (0..n)
                            .map(|i| {
                                if unordered {
                                    ((i * 37 + 1) % n) as f64
                                } else {
                                    i as f64
                                }
                            })
                            .collect(),
                    )?;
                    let v = Matrix::from_column_vec(
                        n,
                        1,
                        x.data().iter().map(|x| 2. * x + 1.).collect(),
                    )?;
                    let q = Matrix::from_column_vec(
                        1,
                        m,
                        (0..m)
                            .map(|i| match i % 16 {
                                0 => -1.,
                                1 => n as f64,
                                2 => (i % n) as f64,
                                _ => ((i * 37) % ((n - 1) * 4 + 1)) as f64 / 4.,
                            })
                            .collect(),
                    )?;
                    let aliases = (x.clone(), v.clone(), q.clone());
                    let mut expected_checksum = None;
                    for rep in 0..9 {
                        for workers in workers {
                            let pool = &pools[usize::from(workers == 8)];
                            let start = Instant::now();
                            let result = pool.install(|| {
                                interpolate_vectors(black_box(&x), &v, &q, method, -7., &cancel)
                            })?;
                            let ns = start.elapsed().as_nanos();
                            assert_eq!((result.rows(), result.cols()), (1, m));
                            let mut hash = blake3::Hasher::new();
                            for (&query, &actual) in q.data().iter().zip(result.data()) {
                                let expected = if query < 0. || query > (n - 1) as f64 {
                                    -7.
                                } else {
                                    2. * match method {
                                        InterpolationMethod::Linear => query,
                                        InterpolationMethod::Nearest => (query + 0.5).floor(),
                                    } + 1.
                                };
                                assert_eq!(actual.to_bits(), expected.to_bits());
                                hash.update(&actual.to_bits().to_le_bytes());
                            }
                            let checksum = hash.finalize().to_hex().to_string();
                            if let Some(expected) = &expected_checksum {
                                assert_eq!(&checksum, expected);
                            } else {
                                expected_checksum = Some(checksum.clone());
                            }
                            assert_eq!(x.data().as_ptr(), aliases.0.data().as_ptr());
                            assert_eq!(v.data().as_ptr(), aliases.1.data().as_ptr());
                            assert_eq!(q.data().as_ptr(), aliases.2.data().as_ptr());
                            println!(
                                "{},{case},{n},{m},{method:?},{unordered},{workers},{rep},{},{ns},{checksum}",
                                args[1],
                                u8::from(rep < 2)
                            );
                        }
                    }
                    case += 1;
                }
            }
        }
    }
    assert_eq!(case, 64);
    Ok(())
}
