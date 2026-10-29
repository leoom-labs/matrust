//! Serial public API baseline. Requires a separately reviewed independent reference TSV.
use matrust::{
    Matrix,
    array::{expm1, log1p},
};
use std::{hint::black_box, io::Read, sync::atomic::AtomicBool, time::Instant};

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
                if input_bits != value.to_bits() || max_ulp > 2 || f64::from_bits(expected).is_nan()
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

fn main() -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("benchmark_near_zero requires release".into());
    }
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 || !matches!(args[1].as_str(), "normal" | "reverse") {
        return Err("benchmark_near_zero normal|reverse INDEPENDENT_REFERENCE.tsv".into());
    }
    let (references, reference_digest) = reference_file(&args[2])?;
    let workers = if args[1] == "normal" { [1, 8] } else { [8, 1] };
    let pools = [1, 8].map(|n| rayon::ThreadPoolBuilder::new().num_threads(n).build());
    let [one, eight] = pools;
    let pools = [
        one.map_err(|e| e.to_string())?,
        eight.map_err(|e| e.to_string())?,
    ];
    for (pool, count) in pools.iter().zip([1, 8]) {
        pool.install(|| assert_eq!(rayon::current_num_threads(), count));
    }
    let cancel = AtomicBool::new(false);
    println!("# serial correctness baseline; independent_reference_blake3={reference_digest}");
    println!("# 24 cases x 2 contexts x 9 repetitions = 432 rows/order; 336 retained, 96 warm");
    println!(
        "# complete public call + pool.install timed; input construction, external-reference validation, checksum and destruction excluded"
    );
    println!("order,case,function,corpus,elements,workers,rep,warmup,ns,checksum");
    let mut case = 0;
    for (operation, name) in NAMES.iter().enumerate() {
        for (corpus, corpus_name) in CORPORA.iter().enumerate() {
            let pattern = pattern(operation, corpus);
            for n in [32usize, 4096, 65536, 1048576] {
                let input =
                    Matrix::from_column_vec(1, n, (0..n).map(|i| pattern[i % 8]).collect())?;
                let alias = input.clone();
                let mut expected_digest = None;
                for rep in 0..9 {
                    for workers in workers {
                        let start = Instant::now();
                        let output = black_box(pools[usize::from(workers == 8)].install(|| {
                            if operation == 0 {
                                log1p(black_box(&input), &cancel)
                            } else {
                                expm1(black_box(&input), &cancel)
                            }
                        })?);
                        let ns = start.elapsed().as_nanos();
                        assert_eq!((output.rows(), output.cols()), (1, n));
                        let mut hash = blake3::Hasher::new();
                        hash.update(&1u64.to_le_bytes());
                        hash.update(&(n as u64).to_le_bytes());
                        for (i, &actual) in output.data().iter().enumerate() {
                            assert!(
                                check_value(actual, references[operation * 3 + corpus][i % 8]),
                                "reference mismatch: {name} {corpus_name} index={i}"
                            );
                            hash.update(&actual.to_bits().to_le_bytes());
                        }
                        let digest = hash.finalize().to_hex().to_string();
                        if let Some(expected) = &expected_digest {
                            assert_eq!(&digest, expected);
                        } else {
                            expected_digest = Some(digest.clone());
                        }
                        assert_eq!(input.data().as_ptr(), alias.data().as_ptr());
                        println!(
                            "{},{case},{name},{corpus_name},{n},{workers},{rep},{},{ns},{digest}",
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
