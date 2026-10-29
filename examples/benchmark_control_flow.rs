//! End-to-end script timing: parse, execute, workspace writes and output destruction.
use matrust::Engine;
use std::{hint::black_box, time::Instant};
fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    println!("case,iterations,threads,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for n in [100usize, 10000, 100000] {
                for (name, source, expected) in [
                    (
                        "range_sum",
                        format!("s=0;for k=1:{n};s=s+k;end"),
                        (n * (n + 1) / 2) as f64,
                    ),
                    (
                        "while_sum",
                        format!("s=0;k=0;while k<{n};k=k+1;s=s+k;end"),
                        (n * (n + 1) / 2) as f64,
                    ),
                    ("range_empty", format!("for k=1:{n};end;s=k;"), n as f64),
                    (
                        "range_alias",
                        format!("for k=1:{n};saved=k;end;s=saved;"),
                        n as f64,
                    ),
                ] {
                    let mut e = Engine::default();
                    e.execute(&source).unwrap();
                    assert_eq!(e.variables["s"].data(), &[expected]);
                    let mut times = Vec::new();
                    for repetition in 0..14 {
                        let start = Instant::now();
                        black_box(e.execute(black_box(&source)).unwrap());
                        if repetition >= 3 {
                            times.push(start.elapsed().as_secs_f64() * 1000.);
                        }
                    }
                    times.sort_by(f64::total_cmp);
                    println!(
                        "{name},{n},{workers},11,{:.6},{:.6},{:.6}",
                        times[5], times[0], times[10]
                    );
                }
            }
            let source = include_str!("control_flow_filtering.m");
            let mut times = Vec::new();
            for repetition in 0..14 {
                let mut e = Engine::default();
                let start = Instant::now();
                black_box(e.execute(black_box(source)).unwrap());
                if repetition >= 3 {
                    times.push(start.elapsed().as_secs_f64() * 1000.);
                }
                assert_eq!(e.variables["chunks_run"].data(), &[24.]);
            }
            times.sort_by(f64::total_cmp);
            println!(
                "filter_script,24,{workers},11,{:.6},{:.6},{:.6}",
                times[5], times[0], times[10]
            );
        });
}
