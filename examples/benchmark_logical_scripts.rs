//! Release script-path measurements. Run separately from compilation and MATLAB.
use matrust::{Engine, LogicalMatrix as L, Matrix};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    let threads: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&threads));
    println!("case,elements,threads,repetitions,batch,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(|| {
            let flag = AtomicBool::new(false);
            for n in [1024, 524288, 1048576, 8388608] {
                let source = Matrix::from_column_vec(
                    n / 8,
                    8,
                    (0..n).map(|i| i as f64 - 0.5 * (n as f64)).collect(),
                )
                .unwrap();
                let mask = L::from_column_vec(
                    n / 8,
                    8,
                    (0..n).map(|i| u8::from(i % 2 == 0)).collect(),
                    &flag,
                )
                .unwrap();
                let all = L::filled(n / 8, 8, true, &flag).unwrap();
                let mut e = Engine::default();
                e.variables.insert("A".into(), source.clone());
                e.logical_variables.insert("M".into(), mask.clone());
                e.logical_variables.insert("T".into(), all);
                for (name, script) in [
                    ("compare", "R=A>0;"),
                    ("numeric_gather", "R=A(M);"),
                    ("identity_mask", "R=A(T);"),
                    ("logical_gather", "R=M(M);"),
                    ("numeric_mask_write", "A(M)=0;"),
                    ("logical_mask_write", "M(M)=false;"),
                    ("transpose", "R=M';"),
                    ("nnz", "R=nnz(M);"),
                    ("find", "R=find(M);"),
                    ("all", "R=all(T);"),
                ] {
                    let reset = |e: &mut Engine| {
                        if name == "numeric_mask_write" {
                            e.variables.insert("A".into(), source.clone());
                        }
                        if name == "logical_mask_write" {
                            e.logical_variables.insert("M".into(), mask.clone());
                        }
                    };
                    reset(&mut e);
                    e.execute(script).unwrap();
                    // Independent result checks before timing.
                    if name == "numeric_mask_write" {
                        for (i, &x) in e.variables["A"].data().iter().enumerate() {
                            assert_eq!(x, if i % 2 == 0 { 0. } else { source.data()[i] });
                        }
                    } else if name == "logical_mask_write" {
                        assert!(e.logical_variables["M"].data().iter().all(|v| *v == 0));
                    } else {
                        let a = e.array("R").unwrap();
                        for c in 0..a.cols() {
                            for r in 0..a.rows() {
                                let i = c * a.rows() + r;
                                let expected = match name {
                                    "compare" => f64::from(source.data()[i] > 0.),
                                    "numeric_gather" => source.data()[2 * i],
                                    "identity_mask" => source.data()[i],
                                    "logical_gather" | "all" => 1.,
                                    "transpose" => f64::from(mask.get(c, r)),
                                    "nnz" => (n / 2) as f64,
                                    "find" => (2 * i + 1) as f64,
                                    _ => unreachable!(),
                                };
                                assert_eq!(
                                    a.scalar(r, c).unwrap().into_real().unwrap(),
                                    expected,
                                    "{name}"
                                );
                            }
                        }
                    }
                    for _ in 0..3 {
                        reset(&mut e);
                        black_box(e.execute(script).unwrap());
                    }
                    let batch = if n == 1024 { 20 } else { 1 };
                    let mut times = Vec::new();
                    for _ in 0..21 {
                        let mut elapsed = 0.;
                        for _ in 0..batch {
                            reset(&mut e);
                            let start = Instant::now();
                            black_box(e.execute(black_box(script)).unwrap());
                            elapsed += start.elapsed().as_secs_f64() * 1000.;
                        }
                        times.push(elapsed / batch as f64);
                    }
                    times.sort_by(f64::total_cmp);
                    println!(
                        "{name},{n},{threads},21,{batch},{:.6},{:.6},{:.6}",
                        times[10], times[0], times[20]
                    );
                    e.variables.insert("A".into(), source.clone());
                    e.logical_variables.insert("M".into(), mask.clone());
                }
            }
        });
}
