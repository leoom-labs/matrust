//! Fixed whole-public-call context measurement; no production parallel strategy.
//! Correctness references live in tests/set_operations.rs. Timed results must
//! match a precomputed complete-bit baseline; that is not an independent reference.
use matrust::{
    Array, LogicalMatrix, Matrix,
    array::{SetOperation, SetResult, UniqueOrder},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

const SIZES: [usize; 3] = [32, 8192, 524288];
const FAMILIES: [&str; 6] = [
    "ordered_overlap",
    "reverse_disjoint",
    "duplicates",
    "logical",
    "real_logical",
    "logical_real",
];
const OPS: [(SetOperation, &str); 4] = [
    (SetOperation::Union, "union"),
    (SetOperation::Intersect, "intersect"),
    (SetOperation::Difference, "setdiff"),
    (SetOperation::SymmetricDifference, "setxor"),
];
fn data(n: usize, family: usize, side: usize) -> Array {
    let logical = family == 3 || (family == 4 && side == 1) || (family == 5 && side == 0);
    if logical {
        LogicalMatrix::from_column_vec(
            1,
            n,
            (0..n).map(|i| ((i + side) % 2) as u8).collect(),
            &AtomicBool::new(false),
        )
        .unwrap()
        .into()
    } else {
        Matrix::from_column_vec(
            1,
            n,
            (0..n)
                .map(|i| match family {
                    0 => (i + side * (n / 2)) as f64,
                    1 => (n - i + side * n) as f64,
                    2 => ((i * 37 + side * 7) % 17) as f64,
                    4 | 5 => (i % 5) as f64 - 1.,
                    _ => unreachable!(),
                })
                .collect(),
        )
        .unwrap()
        .into()
    }
}
fn hash(result: &SetResult) -> String {
    let mut h = blake3::Hasher::new();
    match &result.values {
        None => {
            h.update(&[0]);
        }
        Some(Array::Numeric(a)) => {
            h.update(&[1]);
            h.update(&(a.rows() as u64).to_le_bytes());
            h.update(&(a.cols() as u64).to_le_bytes());
            for x in a.data() {
                h.update(&x.to_bits().to_le_bytes());
            }
        }
        Some(Array::Logical(a)) => {
            h.update(&[2]);
            h.update(&(a.rows() as u64).to_le_bytes());
            h.update(&(a.cols() as u64).to_le_bytes());
            h.update(a.data());
        }
        _ => unreachable!(),
    }
    for output in [&result.first_a, &result.first_b] {
        if let Some(a) = output {
            h.update(&[1]);
            h.update(&(a.rows() as u64).to_le_bytes());
            h.update(&(a.cols() as u64).to_le_bytes());
            for x in a.data() {
                h.update(&x.to_bits().to_le_bytes());
            }
        } else {
            h.update(&[0]);
        }
    }
    h.finalize().to_hex().to_string()
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("release benchmark only");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(
        args.len() == 1 && matches!(args[0].as_str(), "normal" | "reverse"),
        "expected normal or reverse"
    );
    let reverse = args[0] == "reverse";
    let pools = [1, 8].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
    });
    for (pool, workers) in pools.iter().zip([1, 8]) {
        pool.install(|| assert_eq!(rayon::current_num_threads(), workers));
    }
    let mut cases = Vec::new();
    for op in 0..4 {
        for stable in [false, true] {
            for family in 0..6 {
                for size in 0..3 {
                    for all_outputs in [false, true] {
                        for workers in [1, 8] {
                            cases.push((op, stable, family, size, all_outputs, workers));
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases.len(), 576, "288 fixtures times two worker contexts");
    if reverse {
        cases.reverse();
    }
    println!(
        "process_order,operation,stable,family,size,outputs,workers,rep,warmup,elapsed_ns,checksum"
    );
    for (op, stable, family, size, all_outputs, workers) in cases {
        let n = SIZES[size];
        let a = data(n, family, 0);
        let b = data(n, family, 1);
        let aliases = (a.clone(), b.clone());
        let cancel = AtomicBool::new(false);
        let order = if stable {
            UniqueOrder::Stable
        } else {
            UniqueOrder::Sorted
        };
        let operation = OPS[op].0;
        let mask = [
            true,
            all_outputs,
            all_outputs && operation != SetOperation::Difference,
        ];
        let expected = pools[0].install(|| {
            hash(
                &a.set_operation(&b, operation, order, mask, &cancel)
                    .unwrap(),
            )
        });
        let pool = &pools[usize::from(workers == 8)];
        for rep in 0..9 {
            let start = Instant::now();
            let result = pool
                .install(|| a.set_operation(&b, operation, order, mask, &cancel))
                .unwrap();
            let elapsed = start.elapsed().as_nanos();
            black_box(&result);
            let checksum = hash(&result);
            assert_eq!(checksum, expected);
            assert!(a.same_snapshot(&aliases.0) && b.same_snapshot(&aliases.1));
            println!(
                "{},{},{},{},{},{},{},{},{},{},{}",
                args[0],
                OPS[op].1,
                stable,
                FAMILIES[family],
                n,
                if all_outputs { "all" } else { "values" },
                workers,
                rep,
                rep < 2,
                elapsed,
                checksum
            );
        }
    }
}
