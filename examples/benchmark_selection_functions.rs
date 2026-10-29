//! Selection latency includes validating the containing document and execution.
use matrust::Engine;
use std::{hint::black_box, time::Instant};

fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    println!("case,source_bytes,workers,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let code = "R=helper(7);";
            let helper = "\nfunction y=helper(x)\ny=x+1;\nend";
            let prefix = format!("unselected=missing;\n{code}\n%");
            for size in [1024, 100_000, 1_000_000] {
                let source = format!(
                    "{prefix}{}{helper}",
                    "p".repeat(size - prefix.len() - helper.len())
                );
                let start = source.find(code).unwrap();
                let range = start..start + code.len();
                let equivalent = format!("{code}{helper}");
                for selection in [false, true] {
                    let mut engine = Engine::default();
                    let mut times = Vec::new();
                    for i in 0..34 {
                        let clock = Instant::now();
                        if selection {
                            black_box(
                                engine
                                    .execute_selection(black_box(&source), range.clone())
                                    .unwrap(),
                            );
                        } else {
                            black_box(engine.execute(black_box(&equivalent)).unwrap());
                        }
                        if i >= 3 {
                            times.push(clock.elapsed().as_secs_f64() * 1000.);
                        }
                        assert_eq!(engine.variables["R"].data(), &[8.]);
                        assert!(!engine.variables.contains_key("unselected"));
                    }
                    times.sort_by(f64::total_cmp);
                    println!(
                        "{},{size},{workers},31,{:.6},{:.6},{:.6}",
                        if selection {
                            "selection"
                        } else {
                            "standalone_equivalent"
                        },
                        times[15],
                        times[0],
                        times[30]
                    );
                }
            }
        });
}
