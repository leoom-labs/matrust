use matrust::{
    Array, LogicalMatrix,
    array::{MissingPlacement, SortOrder},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    let reverse = std::env::args().skip(1).any(|a| a == "--reverse");
    let cancel = AtomicBool::new(false);
    let mut workers = vec![1, 8];
    if reverse {
        workers.reverse();
    }
    let mut cases = vec![
        (16384, 1, 1),
        (65536, 1, 1),
        (524287, 1, 1),
        (524289, 1, 1),
        (8193, 64, 1),
        (64, 8193, 2),
        (128, 4096, 1),
    ];
    if reverse {
        cases.reverse();
    }
    let pools: Vec<_> = workers
        .iter()
        .map(|n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(*n)
                .build()
                .unwrap()
        })
        .collect();
    println!("rows,cols,dim,workers,pattern,descending,indices,sample,nanoseconds");
    for (rows, cols, dim) in cases {
        for pattern in ["repeated", "ordered", "reversed"] {
            let a: Array = LogicalMatrix::from_column_vec(
                rows,
                cols,
                (0..rows * cols)
                    .map(|i| match pattern {
                        "ordered" => u8::from(i >= rows * cols / 2),
                        "reversed" => u8::from(i < rows * cols / 2),
                        _ => u8::from(i % 7 < 3),
                    })
                    .collect(),
                &cancel,
            )
            .unwrap()
            .into();
            for descending in [false, true] {
                let (lanes, n) = if dim == 1 { (cols, rows) } else { (rows, cols) };
                // Independent stable reference outside all timed sections.
                let mut expected = vec![0.; a.len()];
                let mut positions = expected.clone();
                for lane in 0..lanes {
                    let mut order: Vec<_> = (0..n).collect();
                    let value = |at| {
                        if dim == 1 {
                            a.scalar(at, lane).unwrap().into_real().unwrap()
                        } else {
                            a.scalar(lane, at).unwrap().into_real().unwrap()
                        }
                    };
                    order.sort_by_key(|at| {
                        if descending {
                            1 - value(*at) as u8
                        } else {
                            value(*at) as u8
                        }
                    });
                    for (at, original) in order.into_iter().enumerate() {
                        let out = if dim == 1 {
                            lane * rows + at
                        } else {
                            lane + at * rows
                        };
                        expected[out] = value(original);
                        positions[out] = original as f64 + 1.;
                    }
                }
                for want_indices in [false, true] {
                    for (pool, workers) in pools.iter().zip(&workers) {
                        let run = || {
                            a.sort_dimension(
                                dim,
                                if descending {
                                    SortOrder::Descend
                                } else {
                                    SortOrder::Ascend
                                },
                                MissingPlacement::Auto,
                                want_indices,
                                &cancel,
                            )
                            .unwrap()
                        };
                        let actual = pool.install(run);
                        assert!(actual.0.is_logical());
                        for (i, value) in expected.iter().enumerate() {
                            assert_eq!(
                                actual
                                    .0
                                    .scalar(i % rows, i / rows)
                                    .unwrap()
                                    .into_real()
                                    .unwrap(),
                                *value
                            );
                        }
                        if want_indices {
                            assert_eq!(actual.1.unwrap().data(), positions);
                        } else {
                            assert!(actual.1.is_none());
                        }
                        for _ in 0..2 {
                            black_box(pool.install(run));
                        }
                        for sample in 0..7 {
                            let start = Instant::now();
                            black_box(pool.install(run));
                            let ns = start.elapsed().as_nanos();
                            println!(
                                "{rows},{cols},{dim},{workers},{pattern},{descending},{want_indices},{sample},{ns}"
                            );
                        }
                    }
                }
            }
        }
    }
}
