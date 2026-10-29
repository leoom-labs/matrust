//! Release-only kernel/threshold measurement, not an interpreter or MATLAB benchmark.
use matrust::{
    LogicalMatrix as L, Matrix as M,
    logical::{ArrayRef as A, BooleanOp as B, Comparison as C, Predicate as P},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn measure(mut f: impl FnMut(), repeats: usize, batch: usize) -> (f64, f64, f64) {
    for _ in 0..3 {
        f();
    }
    let mut times = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let start = Instant::now();
        for _ in 0..batch {
            f();
        }
        times.push(start.elapsed().as_secs_f64() * 1000. / batch as f64);
    }
    times.sort_by(f64::total_cmp);
    (times[repeats / 2], times[0], times[repeats - 1])
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let threads: usize = args.get(1).map_or(8, |v| v.parse().unwrap());
    assert!([1, 8].contains(&threads));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    println!("case,elements,threads,repetitions,batch,median_ms,min_ms,max_ms");
    pool.install(|| {
        let cancel = AtomicBool::new(false);
        for n in [1024, 65536, 262144, 524288, 1048576, 4194304, 8388608] {
            let x =
                M::from_column_vec(n, 1, (0..n).map(|i| ((i * 17) % 37) as f64 - 18.).collect())
                    .unwrap();
            let y = M::from_column_vec(n, 1, (0..n).map(|i| ((i * 7) % 23) as f64 - 11.).collect())
                .unwrap();
            let zero = M::scalar(0.);
            let a = L::compare(A::Numeric(&x), A::Numeric(&zero), C::Greater, &cancel).unwrap();
            let b = L::compare(A::Numeric(&y), A::Numeric(&zero), C::Greater, &cancel).unwrap();
            let flag = black_box(&cancel);
            // Validate entire outputs before timing. Scalar references are independent
            // of the chunked kernels, and include the nonmultiple tails in tests.
            let pair = L::compare(A::Numeric(&x), A::Numeric(&y), C::Less, &cancel).unwrap();
            let mixed = L::compare(A::Numeric(&x), A::Logical(&b), C::Equal, &cancel).unwrap();
            let logical_pair =
                L::compare(A::Logical(&a), A::Logical(&b), C::Less, &cancel).unwrap();
            let p = L::predicate(A::Numeric(&x), P::IsFinite, &cancel).unwrap();
            let converted = L::from_numeric(&x, &cancel).unwrap();
            let and = a.combine(&b, B::And, &cancel).unwrap();
            let not = a.not(&cancel).unwrap();
            let doubles = a.to_numeric(&cancel).unwrap();
            for i in 0..n {
                let xv = x.data()[i];
                let yv = y.data()[i];
                assert_eq!(a.data()[i], u8::from(xv > 0.));
                assert_eq!(b.data()[i], u8::from(yv > 0.));
                assert_eq!(pair.data()[i], u8::from(xv < yv));
                assert_eq!(
                    mixed.data()[i],
                    u8::from(xv == if yv > 0. { 1. } else { 0. })
                );
                assert_eq!(p.data()[i], 1);
                assert_eq!(converted.data()[i], u8::from(xv != 0.));
                assert_eq!(and.data()[i], u8::from(xv > 0. && yv > 0.));
                assert_eq!(logical_pair.data()[i], u8::from(xv <= 0. && yv > 0.));
                assert_eq!(not.data()[i], u8::from(xv <= 0.));
                assert_eq!(doubles.data()[i], if xv > 0. { 1. } else { 0. });
            }
            drop((pair, mixed, p, converted, and, not, doubles, logical_pair));
            let repeats = 21;
            let batch = if n == 1024 { 100 } else { 1 };
            let run = |name: &str, f: &mut dyn FnMut()| {
                let (median, min, max) = measure(f, repeats, batch);
                println!("{name},{n},{threads},{repeats},{batch},{median:.6},{min:.6},{max:.6}");
            };
            run("compare_scalar", &mut || {
                black_box(
                    L::compare(
                        A::Numeric(black_box(&x)),
                        A::Numeric(&zero),
                        C::Greater,
                        flag,
                    )
                    .unwrap(),
                );
            });
            run("compare_arrays", &mut || {
                black_box(
                    L::compare(A::Numeric(black_box(&x)), A::Numeric(&y), C::Less, flag).unwrap(),
                );
            });
            run("compare_mixed", &mut || {
                black_box(
                    L::compare(A::Numeric(black_box(&x)), A::Logical(&b), C::Equal, flag).unwrap(),
                );
            });
            run("compare_logical", &mut || {
                black_box(
                    L::compare(A::Logical(black_box(&a)), A::Logical(&b), C::Less, flag).unwrap(),
                );
            });
            let logical_scalar = L::scalar(true);
            run("compare_logical_scalar", &mut || {
                black_box(
                    L::compare(
                        A::Logical(black_box(&a)),
                        A::Logical(&logical_scalar),
                        C::Equal,
                        flag,
                    )
                    .unwrap(),
                );
            });
            run("isfinite", &mut || {
                black_box(L::predicate(A::Numeric(black_box(&x)), P::IsFinite, flag).unwrap());
            });
            run("from_numeric", &mut || {
                black_box(L::from_numeric(black_box(&x), flag).unwrap());
            });
            run("to_numeric", &mut || {
                black_box(black_box(&a).to_numeric(flag).unwrap());
            });
            run("not", &mut || {
                black_box(black_box(&a).not(flag).unwrap());
            });
            run("and", &mut || {
                black_box(black_box(&a).combine(&b, B::And, flag).unwrap());
            });
            run("filled", &mut || {
                black_box(L::filled(n, 1, true, flag).unwrap());
            });
            // Reference implementation only: this previously available numeric
            // map represents flags as doubles, not as an actual logical type.
            run("f64_flags_reference", &mut || {
                black_box(black_box(&x).map(|v| if v > 0. { 1. } else { 0. }));
            });
        }
    });
}
