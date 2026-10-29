//! Fixed final-public-API dispatch experiment; no vendor or universal speed claim.
use matrust::{Matrix, array::evaluate_polynomial, initialize_worker_pool};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn analytic(coefficients: usize, query: f64) -> f64 {
    if coefficients == 4 {
        return 2. * query * query * query - 3. * query * query + 5.;
    }
    // The fixture is x^(coefficients-1), x in {-1,-1/2,-0,+0,1/2,1}.
    // Evaluate its closed-form dyadic value independently of Horner's method.
    let degree = coefficients - 1;
    let magnitude = if query == 0. {
        0.
    } else if query.abs() == 1. {
        1.
    } else {
        assert_eq!(query.abs(), 0.5);
        1. / (1u64 << degree) as f64
    };
    if query.is_sign_negative() && degree % 2 == 1 {
        -magnitude
    } else {
        magnitude
    }
}

fn validate(
    coefficients: usize,
    x: &Matrix,
    result: &Matrix,
    reference: Option<&Matrix>,
) -> String {
    assert_eq!((result.rows(), result.cols()), (x.rows(), x.cols()));
    if let Some(reference) = reference {
        assert_eq!((reference.rows(), reference.cols()), (x.rows(), x.cols()));
        assert!(
            result
                .data()
                .iter()
                .zip(reference.data())
                .all(|(got, expected)| got.to_bits() == expected.to_bits())
        );
    }
    let mut hash = blake3::Hasher::new();
    hash.update(&(x.rows() as u64).to_le_bytes());
    hash.update(&(x.cols() as u64).to_le_bytes());
    for (&query, &got) in x.data().iter().zip(result.data()) {
        let expected = analytic(coefficients, query);
        if expected == 0. {
            // A power's analytical zero sign can differ from trailing Horner +0.
            // The full-bit comparison above still requires the serial zero bits.
            assert_eq!(got, 0.);
        } else {
            assert_eq!(got.to_bits(), expected.to_bits());
        }
        hash.update(&got.to_bits().to_le_bytes());
    }
    hash.finalize().to_hex().to_string()
}

fn main() -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("benchmark_polyval_dispatch requires release".into());
    }
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2
        || !matches!(
            args[1].as_str(),
            "normal" | "reverse" | "global1" | "global8"
        )
    {
        return Err("benchmark_polyval_dispatch normal|reverse|global1|global8".into());
    }
    let global = matches!(args[1].as_str(), "global1" | "global8");
    let order = match args[1].as_str() {
        "normal" => [1, 8],
        "reverse" => [8, 1],
        "global1" => [1, 1],
        "global8" => [8, 8],
        _ => unreachable!(),
    };
    let contexts = if global { 1 } else { 2 };
    if global {
        assert_eq!(rayon::current_thread_index(), None);
        initialize_worker_pool(order[0])?;
        // Initialization is complete; this query cannot create another pool.
        assert_eq!(rayon::current_num_threads(), order[0]);
        assert_eq!(rayon::current_thread_index(), None);
    }
    // A global-mode run needs only the local one-worker reference pool.
    let pools: Vec<_> = [1, 8]
        .into_iter()
        .take(contexts)
        .map(|count| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(count)
                .build()
                .unwrap()
        })
        .collect();
    for (pool, count) in pools.iter().zip([1, 8]) {
        pool.install(|| {
            assert!(rayon::current_thread_index().is_some());
            assert_eq!(rayon::current_num_threads(), count);
        });
    }
    let cancel = AtomicBool::new(false);
    let samples = [-1., -0.5, -0., 0., 0.5, 1.];
    if global {
        println!(
            "# 36 fixtures x 1 global context x 9 repetitions = 324 rows/process; 252 retained, 72 warm"
        );
        println!(
            "# direct public kernel timed on main thread; setup/reference/checks/hash/destruction excluded"
        );
    } else {
        println!(
            "# 36 fixtures x 2 contexts x 9 repetitions = 648 rows/order; 504 retained, 144 warm"
        );
        println!(
            "# full public kernel and pool.install timed; input/reference/checks/hash/destruction excluded"
        );
    }
    println!("order,case,coefficients,queries,matrix,workers,rep,warmup,ns,checksum");
    let mut case = 0;
    let mut rows = 0;
    for coefficients in [4usize, 5, 8, 12, 16, 17] {
        for n in [262143usize, 262144, 262145, 1048576] {
            for matrix in [false, true] {
                if matrix && n % 2 != 0 {
                    continue;
                }
                let mut values = vec![0.; coefficients];
                values[0] = 1.;
                if coefficients == 4 {
                    values.copy_from_slice(&[2., -3., 0., 5.]);
                }
                let p = Matrix::from_column_vec(1, coefficients, values)?;
                let x = Matrix::from_column_vec(
                    if matrix { 2 } else { 1 },
                    if matrix { n / 2 } else { n },
                    (0..n).map(|i| samples[i % samples.len()]).collect(),
                )?;
                let aliases = (p.clone(), x.clone());
                // Local one-worker install overrides any registered global pool.
                let reference = pools[0].install(|| evaluate_polynomial(&p, &x, &cancel))?;
                let expected_hash = validate(coefficients, &x, &reference, None);
                for rep in 0..9 {
                    for workers in order.into_iter().take(contexts) {
                        if global {
                            assert_eq!(rayon::current_thread_index(), None);
                        }
                        let start = Instant::now();
                        let result = if global {
                            black_box(evaluate_polynomial(black_box(&p), &x, &cancel)?)
                        } else {
                            black_box(
                                pools[usize::from(workers == 8)]
                                    .install(|| evaluate_polynomial(black_box(&p), &x, &cancel))?,
                            )
                        };
                        let ns = start.elapsed().as_nanos();
                        if global {
                            assert_eq!(rayon::current_thread_index(), None);
                        }
                        let digest = validate(coefficients, &x, &result, Some(&reference));
                        assert_eq!(digest, expected_hash);
                        assert_eq!(p.data().as_ptr(), aliases.0.data().as_ptr());
                        assert_eq!(x.data().as_ptr(), aliases.1.data().as_ptr());
                        println!(
                            "{},{case},{coefficients},{n},{matrix},{workers},{rep},{},{ns},{digest}",
                            args[1],
                            usize::from(rep < 2)
                        );
                        rows += 1;
                    }
                }
                case += 1;
            }
        }
    }
    assert_eq!(case, 36);
    assert_eq!(rows, 324 * contexts);
    Ok(())
}
