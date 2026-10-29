//! Function call/workspace/file-cache costs, including parser and result disposal.
use matrust::{Engine, Matrix};
use std::{hint::black_box, time::Instant};

fn measure(name: &str, size: usize, workers: usize, engine: &mut Engine, source: &str) {
    let mut times = Vec::new();
    for iteration in 0..14 {
        let start = Instant::now();
        black_box(engine.execute(black_box(source)).unwrap());
        if iteration >= 3 {
            times.push(start.elapsed().as_secs_f64() * 1000.);
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{name},{size},{workers},11,{:.6},{:.6},{:.6}",
        times[5], times[0], times[10]
    );
}
fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    println!("case,size,threads,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let folder = tempfile::tempdir().unwrap();
            std::fs::write(
                folder.path().join("accumulate.m"),
                "function y=accumulate(a,b);y=a+b;end",
            )
            .unwrap();
            for n in [100usize, 10000, 100000] {
                for files in [false, true] {
                    let mut e = Engine::default();
                    let mut source = format!("s=0;for k=1:{n};s=accumulate(s,k);end;");
                    if files {
                        e.set_function_directory(folder.path()).unwrap();
                    } else {
                        source.push_str("function y=accumulate(a,b);y=a+b;end");
                    }
                    e.execute(&source).unwrap();
                    assert_eq!(e.variables["s"].data(), &[(n * (n + 1) / 2) as f64]);
                    measure(
                        if files { "file_sum" } else { "local_sum" },
                        n,
                        workers,
                        &mut e,
                        &source,
                    );
                }
            }
            for elements in [1024, 8388608] {
                let mut e = Engine::default();
                e.variables
                    .insert("A".into(), Matrix::filled(1, elements, 7.).unwrap());
                let source = "B=identity(A);function y=identity(x);y=x;end";
                e.execute(source).unwrap();
                assert_eq!(
                    e.variables["A"].data().as_ptr(),
                    e.variables["B"].data().as_ptr()
                );
                measure("shared_identity", elements, workers, &mut e, source);
                assert_eq!(
                    e.variables["A"].data().as_ptr(),
                    e.variables["B"].data().as_ptr()
                );
            }
            let mut e = Engine::default();
            let recursive = "R=fact(15);function y=fact(n);if n<=1;y=1;else;y=n*fact(n-1);end;end";
            e.execute(recursive).unwrap();
            assert_eq!(e.variables["R"].data(), &[1307674368000.]);
            measure("factorial", 15, workers, &mut e, recursive);
            let source = include_str!("user_functions.m");
            let mut e = Engine::default();
            e.execute(source).unwrap();
            assert!(e.variables["error"].data()[0] < 1e-13);
            measure("filter_script", 2000, workers, &mut e, source);
        });
}
