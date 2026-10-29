//! Fixed complete-call serial-production context benchmark. Root-owned release runs only.
use matrust::{Array, LogicalMatrix, Matrix, array::MembershipResult};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

const SIZES: [(usize, usize); 4] = [(32, 32), (4096, 4096), (262144, 65536), (1048576, 1048576)];
const FAMILIES: [&str; 8] = [
    "ordered",
    "reverse",
    "duplicates",
    "ieee",
    "disjoint",
    "logical",
    "real_logical",
    "logical_real",
];
fn ieee(i: usize) -> f64 {
    [
        f64::NAN,
        -0.,
        0.,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::from_bits(1),
        -f64::from_bits(1),
        f64::MAX,
        -f64::MAX,
        1.,
        f64::NAN,
        2.,
        1.,
        0.,
        f64::NAN,
        3.,
    ][i % 16]
}
fn corpus(m: usize, n: usize, family: usize) -> (Array, Array) {
    let cancel = AtomicBool::new(false);
    let a = if family == 5 || family == 7 {
        LogicalMatrix::from_column_vec(1, m, (0..m).map(|i| (i % 2) as u8).collect(), &cancel)
            .unwrap()
            .into()
    } else {
        Matrix::from_column_vec(
            1,
            m,
            (0..m)
                .map(|i| match family {
                    0 | 1 => (i % (n + 1)) as f64,
                    2 => (i % 17) as f64,
                    3 => ieee(i),
                    4 => (n + i % n) as f64,
                    6 => (i % 3) as f64 - 1.,
                    _ => unreachable!(),
                })
                .collect(),
        )
        .unwrap()
        .into()
    };
    let b = if family == 5 || family == 6 {
        LogicalMatrix::from_column_vec(n, 1, (0..n).map(|i| (i % 2) as u8).collect(), &cancel)
            .unwrap()
            .into()
    } else {
        Matrix::from_column_vec(
            n,
            1,
            (0..n)
                .map(|i| match family {
                    0 | 4 => i as f64,
                    1 => (n - 1 - i) as f64,
                    2 => (i % 16) as f64,
                    3 => ieee(i),
                    7 => [2., 3., -0., 4., 1.][i % 5],
                    _ => unreachable!(),
                })
                .collect(),
        )
        .unwrap()
        .into()
    };
    (a, b)
}
fn analytic_index(i: usize, n: usize, family: usize) -> usize {
    match family {
        0 => {
            let q = i % (n + 1);
            if q < n { q + 1 } else { 0 }
        }
        1 => {
            let q = i % (n + 1);
            n.saturating_sub(q)
        }
        2 => {
            let q = i % 17;
            if q < 16 { q + 1 } else { 0 }
        }
        3 => [0, 2, 2, 4, 5, 6, 7, 8, 9, 10, 0, 12, 10, 2, 0, 16][i % 16],
        4 => 0,
        5 => i % 2 + 1,
        6 => [0, 1, 2][i % 3],
        7 => [3, 5][i % 2],
        _ => unreachable!(),
    }
}
fn validate(result: &MembershipResult, m: usize, n: usize, family: usize, all: bool) -> String {
    let lia = result.membership.as_ref().unwrap();
    assert_eq!((lia.rows(), lia.cols()), (1, m));
    assert_eq!(result.locations.is_some(), all);
    if let Some(loc) = &result.locations {
        assert_eq!((loc.rows(), loc.cols()), (1, m));
    }
    let mut hash = blake3::Hasher::new();
    for (i, &actual) in lia.data().iter().enumerate() {
        let expected = analytic_index(i, n, family);
        assert_eq!(actual, u8::from(expected != 0));
        hash.update(&[actual]);
        if let Some(loc) = &result.locations {
            let bits = loc.data()[i].to_bits();
            assert_eq!(bits, (expected as f64).to_bits());
            hash.update(&bits.to_le_bytes());
        }
    }
    hash.finalize().to_hex().to_string()
}

fn main() -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("benchmark_ismember requires release".into());
    }
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2 || !matches!(args[1].as_str(), "normal" | "reverse") {
        return Err("benchmark_ismember normal|reverse".into());
    }
    let order = args[1].as_str();
    let workers = if order == "normal" { [1, 8] } else { [8, 1] };
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    for (i, pool) in pools.iter().enumerate() {
        pool.install(|| assert_eq!(rayon::current_num_threads(), if i == 0 { 1 } else { 8 }));
    }
    let cancel = AtomicBool::new(false);
    println!(
        "# production is serial in both caller pools; complete-call validation/sort/allocation included; analytic checks/hash excluded"
    );
    println!("# 64 cases, 2 warm + 7 retained: 1152 rows/order, 896 retained, 256 warm");
    println!("order,case,m,n,family,outputs,workers,rep,warmup,ns,checksum");
    let mut case = 0;
    for (m, n) in SIZES {
        for (family, name) in FAMILIES.iter().enumerate() {
            let (a, b) = corpus(m, n, family);
            let aliases = (a.clone(), b.clone());
            for all in [false, true] {
                let mut checksum = None;
                for rep in 0..9 {
                    for workers in workers {
                        let start = Instant::now();
                        let result = pools[usize::from(workers == 8)].install(|| {
                            black_box(&a).membership(black_box(&b), [true, all], &cancel)
                        })?;
                        let ns = start.elapsed().as_nanos();
                        let observed = validate(&result, m, n, family, all);
                        if let Some(previous) = &checksum {
                            assert_eq!(previous, &observed);
                        } else {
                            checksum = Some(observed.clone());
                        }
                        assert!(a.same_snapshot(&aliases.0) && b.same_snapshot(&aliases.1));
                        println!(
                            "{order},{case},{m},{n},{name},{},{workers},{rep},{},{ns},{observed}",
                            if all { 2 } else { 1 },
                            u8::from(rep < 2)
                        );
                    }
                }
                case += 1;
            }
        }
    }
    assert_eq!(case, 64);
    Ok(())
}
