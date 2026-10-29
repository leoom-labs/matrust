//! PRIVATE experiment: public serial baseline versus explicit existing-pool parallel mapping.
//! No production dispatch or speed/accuracy acceptance is implied.
use matrust::{
    Matrix,
    array::{expm1, log1p},
};
use rayon::prelude::*;
use std::{
    hint::black_box,
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

const NAMES: [&str; 2] = ["log1p", "expm1"];
const CORPORA: [&str; 3] = ["tiny", "ordinary", "boundary"];

fn pattern(operation: usize, corpus: usize) -> [f64; 8] {
    match corpus {
        0 => {
            let tiny = f64::from_bits((1023 - 100) << 52);
            let small = f64::from_bits((1023 - 54) << 52);
            [
                0.,
                -0.,
                tiny,
                -tiny,
                small,
                -small,
                f64::from_bits(1),
                -f64::from_bits(1),
            ]
        }
        1 => [-0.75, -0.5, -0.25, 0.25, 0.5, 1., 2., 8.],
        2 if operation == 0 => [
            -1.,
            f64::from_bits((-1.0f64).to_bits() - 1),
            -0.999,
            16.,
            1024.,
            65536.,
            f64::MAX / 2.,
            f64::MAX,
        ],
        2 => [-f64::MAX, -1024., -40., -37., 36., 700., 710., f64::MAX],
        _ => unreachable!(),
    }
}

#[derive(Clone, Copy, Default)]
struct Reference {
    expected: u64,
    max_ulp: u64,
}

fn reference_file(path: &str) -> Result<([[Reference; 8]; 6], String), String> {
    // The file is external, independently produced and root-reviewed. This
    // parser validates its fixed shape, not its mathematical provenance.
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("Reference TSV exceeds 64 KiB".into());
    }
    let digest = blake3::hash(&bytes).to_hex().to_string();
    let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    let mut lines = text.lines();
    if lines.next() != Some("function\tcorpus\tindex\tinput_bits\texpected_bits\tmax_ulp") {
        return Err("Unexpected reference TSV header".into());
    }
    let mut references = [[Reference::default(); 8]; 6];
    for (operation, name) in NAMES.iter().enumerate() {
        for (corpus, corpus_name) in CORPORA.iter().enumerate() {
            for (index, value) in pattern(operation, corpus).iter().enumerate() {
                let fields: Vec<_> = lines
                    .next()
                    .ok_or("Missing reference row")?
                    .split('\t')
                    .collect();
                if fields.len() != 6
                    || fields[0] != *name
                    || fields[1] != *corpus_name
                    || fields[2] != index.to_string()
                {
                    return Err("Unexpected reference row identity".into());
                }
                let input_bits = u64::from_str_radix(fields[3], 16).map_err(|e| e.to_string())?;
                let expected = u64::from_str_radix(fields[4], 16).map_err(|e| e.to_string())?;
                let max_ulp: u64 = fields[5]
                    .parse()
                    .map_err(|e: std::num::ParseIntError| e.to_string())?;
                let expected_value = f64::from_bits(expected);
                let fixed_limit = if expected_value == 0. || expected_value.is_infinite() {
                    0
                } else {
                    2
                };
                if input_bits != value.to_bits()
                    || max_ulp != fixed_limit
                    || expected_value.is_nan()
                {
                    return Err("Reference bits or tolerance outside fixed contract".into());
                }
                references[operation * 3 + corpus][index] = Reference { expected, max_ulp };
            }
        }
    }
    if lines.next().is_some() {
        return Err("Unexpected extra reference row".into());
    }
    Ok((references, digest))
}

fn ordered(bits: u64) -> u64 {
    if bits >> 63 != 0 {
        !bits
    } else {
        bits | (1 << 63)
    }
}

fn check_value(actual: f64, reference: Reference) -> bool {
    let expected = f64::from_bits(reference.expected);
    if expected == 0. || !expected.is_finite() {
        return actual.to_bits() == reference.expected;
    }
    actual.is_finite()
        && ordered(actual.to_bits()).abs_diff(ordered(reference.expected)) <= reference.max_ulp
}

const CHUNK: usize = 4096;
const SIZES: [usize; 9] = [32, 4095, 4096, 4097, 65535, 65536, 65537, 262144, 1048576];

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

fn serial(operation: usize, input: &Matrix, cancel: &AtomicBool) -> Result<Matrix, String> {
    match operation {
        0 => log1p(input, cancel),
        1 => expm1(input, cancel),
        _ => unreachable!(),
    }
}

// Experimental route only. The harness supplies local pools; an ordinary
// off-worker caller is rejected before querying current_num_threads, so this
// function never initializes or constructs a pool. No production threshold.
fn parallel(operation: usize, input: &Matrix, cancel: &AtomicBool) -> Result<Matrix, String> {
    cancelled(cancel)?;
    Matrix::checked_len(input.rows(), input.cols())?;
    if rayon::current_thread_index().is_none() {
        return Err("Parallel prototype requires an existing caller Rayon pool".into());
    }
    let workers = rayon::current_num_threads();
    if !matches!(workers, 1 | 8) {
        return Err("Parallel prototype is restricted to one/eight-worker experiments".into());
    }
    if operation == 0 {
        for chunk in input.data().chunks(CHUNK) {
            cancelled(cancel)?;
            if chunk.iter().any(|&value| value < -1.) {
                return Err("log1p requires real values >= -1 or NaN; complex results for values below -1 are unsupported".into());
            }
        }
    }
    cancelled(cancel)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(input.len())
        .map_err(|_| "Cannot allocate near-zero function output")?;
    cancelled(cancel)?;
    // Safe disjoint mutable slices require initialized storage. This extra zero
    // fill is inside timing, uses the same single reserved payload buffer and
    // checks cancellation every4096 elements. The public serial path extends.
    while output.len() < input.len() {
        cancelled(cancel)?;
        output.resize((output.len() + CHUNK).min(input.len()), 0.);
    }
    let task = input.len().div_ceil(workers * 4).max(CHUNK);
    output
        .par_chunks_mut(task)
        .zip(input.data().par_chunks(task))
        .try_for_each(|(out, source)| -> Result<(), String> {
            for (out, source) in out.chunks_mut(CHUNK).zip(source.chunks(CHUNK)) {
                cancelled(cancel)?;
                match operation {
                    0 => {
                        for (dst, &src) in out.iter_mut().zip(source) {
                            *dst = libm::log1p(src);
                        }
                    }
                    1 => {
                        for (dst, &src) in out.iter_mut().zip(source) {
                            *dst = libm::expm1(src);
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Ok(())
        })?;
    cancelled(cancel)?;
    Matrix::from_column_vec(input.rows(), input.cols(), output)
}

fn route(
    strategy: usize,
    operation: usize,
    input: &Matrix,
    cancel: &AtomicBool,
) -> Result<Matrix, String> {
    match strategy {
        0 => serial(operation, input, cancel),
        1 => parallel(operation, input, cancel),
        _ => unreachable!(),
    }
}

fn verify_result(
    output: &Matrix,
    input: &Matrix,
    baseline: &Matrix,
    references: &[Reference; 8],
    name: &str,
    corpus: &str,
) -> String {
    assert_eq!((output.rows(), output.cols()), (input.rows(), input.cols()));
    let mut hash = blake3::Hasher::new();
    hash.update(&(output.rows() as u64).to_le_bytes());
    hash.update(&(output.cols() as u64).to_le_bytes());
    for (index, &actual) in output.data().iter().enumerate() {
        assert_eq!(
            actual.to_bits(),
            baseline.data()[index % 8].to_bits(),
            "full-bit serial comparison {name}/{corpus}/{index}"
        );
        assert!(
            check_value(actual, references[index % 8]),
            "MPFR reference mismatch {name}/{corpus}/{index}"
        );
        hash.update(&actual.to_bits().to_le_bytes());
    }
    hash.finalize().to_hex().to_string()
}

fn correctness(pools: &[rayon::ThreadPool; 2], refs: &[[Reference; 8]; 6]) -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    for (operation, name) in NAMES.iter().enumerate() {
        for (corpus, corpus_name) in CORPORA.iter().enumerate() {
            let pattern = pattern(operation, corpus);
            let seed = Matrix::from_column_vec(1, 8, pattern.to_vec())?;
            let baseline = pools[0].install(|| serial(operation, &seed, &cancel))?;
            for (rows, cols) in [
                (0, 0),
                (1, 0),
                (0, 1),
                (0, 3),
                (4, 0),
                (1, 8),
                (8, 1),
                (2, 4),
                (1, 4095),
                (1, 4096),
                (1, 4097),
            ] {
                let input = Matrix::from_column_vec(
                    rows,
                    cols,
                    (0..rows * cols).map(|i| pattern[i % 8]).collect(),
                )?;
                let alias = input.clone();
                for pool in pools {
                    for strategy in [0, 1] {
                        let out = pool.install(|| route(strategy, operation, &input, &cancel))?;
                        let _ = verify_result(
                            &out,
                            &input,
                            &baseline,
                            &refs[operation * 3 + corpus],
                            name,
                            corpus_name,
                        );
                        assert_eq!(input.data().as_ptr(), alias.data().as_ptr());
                        for (i, &x) in input.data().iter().enumerate() {
                            assert_eq!(x.to_bits(), pattern[i % 8].to_bits());
                        }
                        cancel.store(true, Ordering::Relaxed);
                        assert_eq!(
                            pool.install(|| route(strategy, operation, &input, &cancel))
                                .unwrap_err(),
                            "Calculation cancelled"
                        );
                        cancel.store(false, Ordering::Relaxed);
                    }
                }
            }
        }
    }
    for index in [0, 4096, 8192] {
        let mut values = vec![0.; 8193];
        values[index] = -2.;
        let input = Matrix::from_column_vec(1, values.len(), values)?;
        for pool in pools {
            let expected = pool.install(|| serial(0, &input, &cancel)).unwrap_err();
            assert_eq!(
                pool.install(|| parallel(0, &input, &cancel)).unwrap_err(),
                expected
            );
        }
    }
    // IEEE cases absent from the finite MPFR corpus: bit agreement against the
    // same locked libm baseline, not a new independent NaN-payload contract.
    for operation in [0, 1] {
        let values = vec![
            f64::NAN,
            f64::from_bits(0xfff8_0000_0000_0001),
            f64::INFINITY,
            -0.,
        ];
        let input = Matrix::from_column_vec(2, 2, values)?;
        let baseline = pools[0].install(|| serial(operation, &input, &cancel))?;
        for pool in pools {
            let out = pool.install(|| parallel(operation, &input, &cancel))?;
            for (&a, &b) in out.data().iter().zip(baseline.data()) {
                assert_eq!(a.to_bits(), b.to_bits());
            }
        }
    }
    let minus_inf = Matrix::from_column_vec(1, 1, vec![f64::NEG_INFINITY])?;
    for pool in pools {
        for strategy in [0, 1] {
            let out = pool.install(|| route(strategy, 1, &minus_inf, &cancel))?;
            assert_eq!(out.data()[0].to_bits(), (-1.0f64).to_bits());
        }
        assert_eq!(
            pool.install(|| parallel(0, &minus_inf, &cancel))
                .unwrap_err(),
            pool.install(|| serial(0, &minus_inf, &cancel)).unwrap_err()
        );
    }
    println!("NEAR_ZERO_PARALLEL_CORRECTNESS_COMPLETE pre_cancel_only_inflight_pending");
    Ok(())
}

fn main() -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("near-zero experiment requires release".into());
    }
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 || !matches!(args[1].as_str(), "check" | "normal" | "reverse") {
        return Err(
            "benchmark_near_zero_parallel check|normal|reverse ROOT_ACCEPTED_REFERENCE.tsv".into(),
        );
    }
    let (references, reference_digest) = reference_file(&args[2])?;
    let pools = [1, 8].map(|n| rayon::ThreadPoolBuilder::new().num_threads(n).build());
    let [one, eight] = pools;
    let pools = [
        one.map_err(|e| e.to_string())?,
        eight.map_err(|e| e.to_string())?,
    ];
    for (pool, n) in pools.iter().zip([1, 8]) {
        pool.install(|| assert_eq!(rayon::current_num_threads(), n));
    }
    if args[1] == "check" {
        return correctness(&pools, &references);
    }
    let workers = if args[1] == "normal" { [1, 8] } else { [8, 1] };
    let strategies = if args[1] == "normal" { [0, 1] } else { [1, 0] };
    let names = ["public_serial", "prototype_parallel"];
    let cancel = AtomicBool::new(false);
    println!("# independent_reference_blake3={reference_digest}");
    println!(
        "# 54 cases x 2 contexts x 2 strategies x 9 repetitions = 1944 rows/order; 1512 retained, 432 warm"
    );
    println!(
        "# full route plus pool.install timed; prototype zero initialization included; reference/check/hash/drop excluded"
    );
    println!("order,case,function,corpus,elements,workers,strategy,rep,warmup,ns,checksum");
    let mut case = 0;
    for (operation, name) in NAMES.iter().enumerate() {
        for (corpus, corpus_name) in CORPORA.iter().enumerate() {
            let pattern = pattern(operation, corpus);
            let seed = Matrix::from_column_vec(1, 8, pattern.to_vec())?;
            let baseline = pools[0].install(|| serial(operation, &seed, &cancel))?;
            for n in SIZES {
                let input =
                    Matrix::from_column_vec(1, n, (0..n).map(|i| pattern[i % 8]).collect())?;
                let alias = input.clone();
                let mut expected_digest = None;
                for rep in 0..9 {
                    for worker in workers {
                        for strategy in strategies {
                            let start = Instant::now();
                            let output =
                                black_box(pools[usize::from(worker == 8)].install(|| {
                                    route(strategy, operation, black_box(&input), &cancel)
                                })?);
                            let ns = start.elapsed().as_nanos();
                            let digest = verify_result(
                                &output,
                                &input,
                                &baseline,
                                &references[operation * 3 + corpus],
                                name,
                                corpus_name,
                            );
                            if let Some(expected) = &expected_digest {
                                assert_eq!(&digest, expected);
                            } else {
                                expected_digest = Some(digest.clone());
                            }
                            assert_eq!(input.data().as_ptr(), alias.data().as_ptr());
                            println!(
                                "{},{case},{name},{corpus_name},{n},{worker},{},{rep},{},{ns},{digest}",
                                args[1],
                                names[strategy],
                                usize::from(rep < 2)
                            );
                        }
                    }
                }
                case += 1;
            }
        }
    }
    assert_eq!(case, 54);
    Ok(())
}
