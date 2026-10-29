//! Provisional storage-kernel admission benchmark, no MATLAB baseline.
//! Setup, output validation and output destruction are outside total_elapsed_ns.
//! Vector transpose aliases use a fixed batch; all outputs survive the timer.
//! Normalized times include bounded loop/push/Arc-retain costs, not destruction.
use matrust::{Complex64, ComplexMatrix};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn run(a: &ComplexMatrix, op: &str, cancel: &AtomicBool) -> usize {
    // Each branch retains the actual result through elapsed capture in measure.
    match op {
        "copy" => ComplexMatrix::from_column_slice(a.rows(), a.cols(), a.data(), cancel)
            .unwrap()
            .len(),
        "real" => a.real(cancel).unwrap().len(),
        "imag" => a.imag(cancel).unwrap().len(),
        "conj" => a.conj(cancel).unwrap().len(),
        "transpose" => a.transpose(cancel).unwrap().len(),
        "adjoint" => a.adjoint(cancel).unwrap().len(),
        _ => unreachable!(),
    }
}
const ALIAS_BATCH: usize = 16_384;

fn alias_case(a: &ComplexMatrix, op: &str) -> bool {
    op == "transpose" && (a.rows() == 1 || a.cols() == 1)
}

fn measure_alias(a: &ComplexMatrix, cancel: &AtomicBool) -> u128 {
    // At most 1 MiB of metadata on this supported 64-bit build. No payload copy.
    if ALIAS_BATCH * std::mem::size_of::<ComplexMatrix>() > 1024 * 1024 {
        panic!("Alias metadata batch exceeds 1 MiB");
    }
    let mut outputs = Vec::with_capacity(ALIAS_BATCH);
    let start = Instant::now();
    for _ in 0..ALIAS_BATCH {
        outputs.push(black_box(black_box(a).transpose(cancel).unwrap()));
    }
    let elapsed = start.elapsed().as_nanos();
    // Shape/storage checks and every Arc decrement occur after elapsed capture.
    assert_eq!(outputs.len(), ALIAS_BATCH);
    for output in &outputs {
        assert_eq!((output.rows(), output.cols()), (a.cols(), a.rows()));
        assert!(output.shares_storage(a));
    }
    drop(outputs);
    elapsed
}

fn measure(a: &ComplexMatrix, op: &str, cancel: &AtomicBool) -> (usize, u128) {
    if alias_case(a, op) {
        return (ALIAS_BATCH, measure_alias(a, cancel));
    }
    (1, measure_materialized(a, op, cancel))
}

fn measure_materialized(a: &ComplexMatrix, op: &str, cancel: &AtomicBool) -> u128 {
    macro_rules! timed {
        ($value:expr) => {{
            let start = Instant::now();
            let output = black_box($value.unwrap());
            let elapsed = start.elapsed().as_nanos();
            assert_eq!(output.len(), a.len());
            drop(output);
            elapsed
        }};
    }
    match op {
        "copy" => timed!(ComplexMatrix::from_column_slice(
            a.rows(),
            a.cols(),
            a.data(),
            cancel
        )),
        "real" => timed!(a.real(cancel)),
        "imag" => timed!(a.imag(cancel)),
        "conj" => timed!(a.conj(cancel)),
        "transpose" => timed!(a.transpose(cancel)),
        "adjoint" => timed!(a.adjoint(cancel)),
        _ => unreachable!(),
    }
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use a release build");
    }
    let reverse = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(reverse.is_empty() || (reverse.len() == 1 && reverse[0] == "--reverse"));
    let reversed = !reverse.is_empty();
    let cancel = AtomicBool::new(false);
    let mut shapes = vec![
        (1, 1024),
        (1024, 1),
        (257, 129),
        (1024, 511),
        (1024, 512),
        (1025, 513),
        (2048, 1024),
    ];
    let mut workers = vec![1, 8];
    let mut operations = vec!["copy", "real", "imag", "conj", "transpose", "adjoint"];
    if reversed {
        shapes.reverse();
        workers.reverse();
        operations.reverse();
    }
    println!("workers,rows,cols,operation,sample,batch_count,total_elapsed_ns,normalized_ns");
    for n in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap();
        for &(rows, cols) in &shapes {
            let a = ComplexMatrix::from_column_vec(
                rows,
                cols,
                (0..rows * cols)
                    .map(|i| Complex64::new((i % 97) as f64, -(i as f64)))
                    .collect(),
                &cancel,
            )
            .unwrap();
            for &op in &operations {
                for _ in 0..2 {
                    if alias_case(&a, op) {
                        let _ = pool.install(|| measure(black_box(&a), op, &cancel));
                    } else {
                        assert_eq!(pool.install(|| run(black_box(&a), op, &cancel)), a.len());
                    }
                }
                for sample in 0..7 {
                    let (batch_count, total_elapsed_ns) =
                        pool.install(|| measure(black_box(&a), op, &cancel));
                    let normalized_ns = total_elapsed_ns as f64 / batch_count as f64;
                    // Keep actual zero totals if observed: the parser rejects them,
                    // preserving raw evidence. No retries, discarded samples or floor.
                    println!(
                        "{n},{rows},{cols},{op},{sample},{batch_count},{total_elapsed_ns},{normalized_ns:.17}"
                    );
                }
            }
            // Full component-bit reference checks are untimed after every shape.
            let t = pool.install(|| a.transpose(&cancel)).unwrap();
            let h = pool.install(|| a.adjoint(&cancel)).unwrap();
            let c = pool.install(|| a.conj(&cancel)).unwrap();
            let re = pool.install(|| a.real(&cancel)).unwrap();
            let im = pool.install(|| a.imag(&cancel)).unwrap();
            let copy = pool
                .install(|| ComplexMatrix::from_column_slice(rows, cols, a.data(), &cancel))
                .unwrap();
            for col in 0..cols {
                for row in 0..rows {
                    let i = col * rows + row;
                    let source = a.data()[i];
                    let trans = t.get(col, row).unwrap();
                    let adj = h.get(col, row).unwrap();
                    assert_eq!(source.re.to_bits(), re.data()[i].to_bits());
                    assert_eq!(source.im.to_bits(), im.data()[i].to_bits());
                    for value in [copy.data()[i], trans] {
                        assert_eq!(
                            [source.re.to_bits(), source.im.to_bits()],
                            [value.re.to_bits(), value.im.to_bits()]
                        );
                    }
                    for value in [c.data()[i], adj] {
                        assert_eq!(
                            [source.re.to_bits(), source.im.to_bits() ^ (1u64 << 63)],
                            [value.re.to_bits(), value.im.to_bits()]
                        );
                    }
                }
            }
        }
    }
}
