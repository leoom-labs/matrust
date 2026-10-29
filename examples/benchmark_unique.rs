//! Fixed release-only full-kernel 1/8 comparison, without a prior unique baseline.
//! Run separately with `normal` and `reverse`; root owns tool/hardware receipts.
use matrust::{
    Array, LogicalMatrix, Matrix,
    array::{UniqueOrder, UniqueResult},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

const SIZES: [usize; 8] = [0, 1, 32, 4096, 65536, 524288, 1048576, 4194304];
const PATTERNS: [&str; 5] = [
    "sorted",
    "reverse",
    "low_cardinality",
    "all_equal",
    "distinct_permutation",
];

fn value(a: &Array, i: usize) -> usize {
    match a {
        Array::Numeric(a) => a.data()[i] as usize,
        Array::Logical(a) => usize::from(a.data()[i]),
        _ => unreachable!(),
    }
}
fn input(n: usize, logical: bool, pattern: usize) -> Array {
    let sample = |i| match pattern {
        0 => i,
        1 => n - 1 - i,
        2 => (i * 37) % 257,
        3 => 7,
        4 => (i * 37) & (n - 1),
        _ => unreachable!(),
    };
    if logical {
        LogicalMatrix::from_column_vec(
            1,
            n,
            (0..n).map(|i| (sample(i) % 2) as u8).collect(),
            &AtomicBool::new(false),
        )
        .unwrap()
        .into()
    } else {
        Matrix::from_column_vec(1, n, (0..n).map(|i| sample(i) as f64).collect())
            .unwrap()
            .into()
    }
}
struct Reference {
    first: Vec<u32>,
    keys: Vec<u32>,
    rank: Vec<u32>,
}
fn reference(a: &Array, order: UniqueOrder) -> Reference {
    // Independent bounded direct-address reference for this integer corpus.
    // No candidate invocation or candidate index comparator generates expected values.
    let domain = (0..a.len()).map(|i| value(a, i)).max().map_or(0, |n| n + 1);
    assert!(domain <= 4194304);
    let mut first = vec![u32::MAX; domain];
    for i in 0..a.len() {
        first[value(a, i)] = first[value(a, i)].min(i as u32);
    }
    let mut keys: Vec<_> = (0..domain as u32)
        .filter(|&key| first[key as usize] != u32::MAX)
        .collect();
    if order == UniqueOrder::Stable {
        keys.sort_by_key(|&key| first[key as usize]);
    }
    let mut rank = vec![0; domain];
    for (i, &key) in keys.iter().enumerate() {
        rank[key as usize] = i as u32 + 1;
    }
    Reference { first, keys, rank }
}
fn validate(a: &Array, result: &UniqueResult, r: &Reference, all: bool) -> String {
    let c = result.values.as_ref().unwrap();
    assert_eq!(c.is_logical(), a.is_logical());
    assert_eq!((c.rows(), c.cols()), (1, r.keys.len()));
    let mut hash = blake3::Hasher::new();
    hash.update(&[u8::from(c.is_logical())]);
    for (i, &key) in r.keys.iter().enumerate() {
        match c {
            Array::Numeric(c) => {
                assert_eq!(c.data()[i].to_bits(), f64::from(key).to_bits());
                hash.update(&c.data()[i].to_bits().to_le_bytes());
            }
            Array::Logical(c) => {
                assert_eq!(u32::from(c.data()[i]), key);
                hash.update(&[c.data()[i]]);
            }
            _ => unreachable!(),
        }
    }
    if all {
        let ia = result.first_indices.as_ref().unwrap();
        let ic = result.group_indices.as_ref().unwrap();
        assert_eq!((ia.rows(), ia.cols()), (r.keys.len(), 1));
        assert_eq!((ic.rows(), ic.cols()), (a.len(), 1));
        for (i, &key) in r.keys.iter().enumerate() {
            assert_eq!(
                ia.data()[i].to_bits(),
                (f64::from(r.first[key as usize]) + 1.).to_bits()
            );
            hash.update(&ia.data()[i].to_bits().to_le_bytes());
        }
        for (i, &actual) in ic.data().iter().enumerate() {
            assert_eq!(actual.to_bits(), f64::from(r.rank[value(a, i)]).to_bits());
            hash.update(&actual.to_bits().to_le_bytes());
        }
    } else {
        assert!(result.first_indices.is_none() && result.group_indices.is_none());
    }
    hash.finalize().to_hex().to_string()
}
fn main() -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("benchmark_unique requires a release build".into());
    }
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2 || !matches!(args[1].as_str(), "normal" | "reverse") {
        return Err("benchmark_unique normal|reverse".into());
    }
    let worker_order = if args[1] == "normal" { [1, 8] } else { [8, 1] };
    let pools = [1, 8].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
    });
    for (at, pool) in pools.iter().enumerate() {
        pool.install(|| assert_eq!(rayon::current_num_threads(), if at == 0 { 1 } else { 8 }));
    }
    let cancel = AtomicBool::new(false);
    println!(
        "# unique full kernel; two prebuilt caller pools; grouping/allocation included; reference/check/hash excluded"
    );
    println!(
        "# 64 fixed cases; 2 warmups + 7 retained; 1152 raw rows per order, 896 retained, 256 warmup; no MATLAB timings"
    );
    println!(
        "# logical patterns are boolean projections of named integer corpora, not all-distinct logical sets"
    );
    println!("order,case,n,class,pattern,mode,outputs,workers,rep,warmup,ns,checksum");
    let mut case = 0;
    for (size_index, &n) in SIZES.iter().enumerate() {
        for logical in [false, true] {
            for (mode_index, mode) in [UniqueOrder::Sorted, UniqueOrder::Stable]
                .into_iter()
                .enumerate()
            {
                for all in [false, true] {
                    // Fixed representative schedule, not the much larger full
                    // distribution cross product. Every size/type/mode/mask is covered.
                    let pattern =
                        (size_index + usize::from(logical) + 2 * mode_index + usize::from(all)) % 5;
                    let a = input(n, logical, pattern);
                    let alias = a.clone();
                    let expected = reference(&a, mode);
                    let mut checksum = None;
                    for rep in 0..9 {
                        for workers in worker_order {
                            let pool = &pools[usize::from(workers == 8)];
                            let start = Instant::now();
                            let result = pool.install(|| {
                                black_box(&a).unique_values(mode, [true, all, all], &cancel)
                            })?;
                            let ns = start.elapsed().as_nanos();
                            let observed = validate(&a, &result, &expected, all);
                            if let Some(old) = &checksum {
                                assert_eq!(old, &observed);
                            } else {
                                checksum = Some(observed.clone());
                            }
                            assert!(a.same_snapshot(&alias));
                            println!(
                                "{},{case},{n},{},{},{},{},{workers},{rep},{},{ns},{observed}",
                                args[1],
                                if logical { "logical" } else { "double" },
                                PATTERNS[pattern],
                                if mode == UniqueOrder::Sorted {
                                    "sorted"
                                } else {
                                    "stable"
                                },
                                if all { "all3" } else { "C" },
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
