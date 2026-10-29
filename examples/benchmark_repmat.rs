//! Bounded block-copy repetition; no parallel threshold is presumed from pools.
use matrust::{Array, Complex64, ComplexMatrix, Engine, LogicalMatrix, Matrix};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(args.iter().all(|x| x == "--reverse"));
    let reverse = args.iter().any(|x| x == "--reverse");
    let mut shapes = vec![
        (2, 3, 1, 1),
        (0, 3, 2, 3),
        (1, 1, 4096, 1),
        (1, 257, 64, 4),
        (4097, 1, 3, 4),
        (3, 5, 4097, 3),
        (2, 3, 1000, 100),
        (1024, 64, 2, 4),
        (1, 8193, 64, 1),
    ];
    let mut types = vec!["real", "logical", "complex"];
    let mut workers = vec![1, 8];
    let mut modes = vec!["kernel", "script"];
    if reverse {
        shapes.reverse();
        types.reverse();
        workers.reverse();
        modes.reverse();
    }
    let cancel = AtomicBool::new(false);
    println!(
        "workers,type,rows,cols,row_copies,column_copies,mode,sample,microseconds,output_bytes"
    );
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for &kind in &types {
            for &(rows, cols, rr, cc) in &shapes {
                let input = match kind {
                    "real" => Array::Numeric(
                        Matrix::from_column_vec(
                            rows,
                            cols,
                            (0..rows * cols).map(|i| i as f64).collect(),
                        )
                        .unwrap(),
                    ),
                    "logical" => Array::Logical(
                        LogicalMatrix::from_column_vec(
                            rows,
                            cols,
                            (0..rows * cols).map(|i| (i % 2) as u8).collect(),
                            &cancel,
                        )
                        .unwrap(),
                    ),
                    _ => Array::Complex(
                        ComplexMatrix::from_column_vec(
                            rows,
                            cols,
                            (0..rows * cols)
                                .map(|i| Complex64::new(i as f64, -(i as f64)))
                                .collect(),
                            &cancel,
                        )
                        .unwrap(),
                    ),
                };
                let mut engine = Engine::default();
                match &input {
                    Array::Numeric(a) => {
                        engine.variables.insert("A".into(), a.clone());
                    }
                    Array::Logical(a) => {
                        engine.logical_variables.insert("A".into(), a.clone());
                    }
                    Array::Complex(a) => {
                        engine.complex_variables.insert("A".into(), a.clone());
                    }
                    Array::Character(_)
                    | Array::Cell(_)
                    | Array::Graphics(_)
                    | Array::Struct(_)
                    | Array::Typed(_)
                    | Array::Handle(_) => {
                        unreachable!("This benchmark constructs only numeric arrays")
                    }
                }
                let script = format!("R=repmat(A,[{rr} {cc}]);");
                for &mode in &modes {
                    for iteration in 0..9 {
                        engine.remove_array("R");
                        let start = Instant::now();
                        let output = pool.install(|| {
                            if mode == "kernel" {
                                black_box(&input).repeated(rr, cc, &cancel)
                            } else {
                                engine
                                    .execute(black_box(&script))
                                    .map(|_| engine.array("R").unwrap())
                            }
                        });
                        let elapsed = start.elapsed();
                        let output = output.unwrap();
                        assert_eq!((output.rows(), output.cols()), (rows * rr, cols * cc));
                        assert_eq!(output.class(), input.class());
                        assert_eq!(output.is_complex(), input.is_complex());
                        if (rr, cc) == (1, 1) {
                            assert!(output.same_snapshot(&input));
                        }
                        // Independent direct coordinate mapping, outside timing.
                        for j in 0..output.cols() {
                            for i in 0..output.rows() {
                                let from = (j % cols) * rows + i % rows;
                                let to = j * output.rows() + i;
                                match (&input, &output) {
                                    (Array::Numeric(a), Array::Numeric(b)) => {
                                        assert_eq!(a.data()[from].to_bits(), b.data()[to].to_bits())
                                    }
                                    (Array::Logical(a), Array::Logical(b)) => {
                                        assert_eq!(a.data()[from], b.data()[to])
                                    }
                                    (Array::Complex(a), Array::Complex(b)) => {
                                        assert_eq!(
                                            a.data()[from].re.to_bits(),
                                            b.data()[to].re.to_bits()
                                        );
                                        assert_eq!(
                                            a.data()[from].im.to_bits(),
                                            b.data()[to].im.to_bits()
                                        );
                                    }
                                    _ => unreachable!(),
                                }
                            }
                        }
                        if iteration >= 2 {
                            println!(
                                "{workers},{kind},{rows},{cols},{rr},{cc},{mode},{},{:.6},{}",
                                iteration - 2,
                                elapsed.as_secs_f64() * 1e6,
                                output.bytes()
                            );
                        }
                        // Result destruction is after timing; next script result
                        // removal and all source construction are also untimed.
                    }
                }
            }
        }
    }
}
