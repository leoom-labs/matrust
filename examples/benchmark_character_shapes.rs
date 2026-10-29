//! Release observations of bounded character metadata versus allocation paths.
use matrust::CharacterRow;
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use release mode");
    }
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(args.is_empty() || (args.len() == 1 && args[0] == "reverse"));
    let reverse = !args.is_empty();
    let stop = AtomicBool::new(false);
    let mut cases = vec![
        (0, 0),
        (1, 0),
        (0, 3),
        (4, 0),
        (2, 3),
        (1, 1023),
        (32, 32),
        (1, 1025),
        (256, 256),
    ];
    if reverse {
        cases.reverse();
    }
    let workers = if reverse { [8, 1] } else { [1, 8] };
    let mut ops = vec!["reshape", "construct", "replace"];
    if reverse {
        ops.reverse();
    }
    println!(
        "rows,cols,workers,operation,repetition,batch,total_ns,ns_per_operation,logical_bytes"
    );
    for worker in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker)
            .build()
            .unwrap();
        for &(rows, cols) in &cases {
            let source = CharacterRow::from_column_major(
                rows,
                cols,
                (0..rows * cols).map(|i| i as u16).collect(),
                &stop,
            )
            .unwrap();
            let alias = source.clone();
            for &op in &ops {
                if op == "replace" && source.is_empty() {
                    continue;
                }
                let batch = if op == "reshape" { 1024 } else { 1 };
                for sample in 0..9 {
                    let mut retained = Vec::with_capacity(batch);
                    let total = pool.install(|| {
                        let start = Instant::now();
                        for _ in 0..batch {
                            let value = match op {
                                "reshape" => source.reshape(1, source.len(), &stop).unwrap(),
                                "construct" => CharacterRow::from_column_major(
                                    rows,
                                    cols,
                                    source.units().to_vec(),
                                    &stop,
                                )
                                .unwrap(),
                                "replace" => {
                                    source.replaced(source.len() / 2, 0xffff, &stop).unwrap()
                                }
                                _ => unreachable!(),
                            };
                            retained.push(black_box(value));
                        }
                        start.elapsed().as_nanos()
                    });
                    for value in &retained {
                        if op == "reshape" {
                            assert!(value.same_storage(&source));
                            assert_eq!((value.rows(), value.cols()), (1, source.len()));
                        } else {
                            assert_eq!((value.rows(), value.cols()), (rows, cols));
                        }
                        for (i, &unit) in value.units().iter().enumerate() {
                            assert_eq!(
                                unit,
                                if op == "replace" && i == source.len() / 2 {
                                    0xffff
                                } else {
                                    source.units()[i]
                                }
                            );
                        }
                    }
                    assert!(source.same_storage(&alias));
                    if sample >= 2 {
                        println!(
                            "{rows},{cols},{worker},{op},{},{batch},{total},{:.9},{}",
                            sample - 2,
                            total as f64 / batch as f64,
                            source.bytes()
                        );
                    }
                    // Retained outputs, validation and destruction are outside the timing interval.
                    black_box(retained);
                }
            }
        }
    }
}
