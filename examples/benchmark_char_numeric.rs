//! Full-engine char conversion/concatenation timing, including parse/assignment.
//! Run: cargo run --release --example benchmark_char_numeric
use matrust::Engine;
use std::{hint::black_box, time::Instant};

fn main() {
    println!("workers,units,operation,repetitions_per_batch,batches,median_us,min_us,max_us");
    for workers in [1, 4] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                for units in [1, 32, 1024, 65536] {
                    let repetitions = if units < 1024 {
                        1000
                    } else if units < 65536 {
                        200
                    } else {
                        40
                    };
                    let mut engine = Engine::default();
                    engine
                        .execute(&format!(
                            "s=char(97*ones(1,{units})); d=double(s); t=char(97*ones(1,{})); lo=char(0); hi=char({});",
                            units / 2, units - 1
                        ))
                        .unwrap();
                    for (operation, source) in [
                        ("double_char", "a=double(s);"),
                        ("char_add", "a=s+0;"),
                        ("double_add", "a=d+0;"),
                        ("char_compare", "a=s==s;"),
                        ("char_abs", "a=abs(s);"),
                        ("char_concat", "a=[t;t];"),
                        ("char_range", "a=lo:hi;"),
                        ("strvcat_padding", "a=strvcat(t,'X');"),
                    ] {
                        for _ in 0..5 {
                            black_box(engine.execute(source).unwrap());
                        }
                        let mut times = Vec::new();
                        for _ in 0..7 {
                            let start = Instant::now();
                            for _ in 0..repetitions {
                                black_box(engine.execute(source).unwrap());
                            }
                            times.push(start.elapsed().as_secs_f64() * 1e6 / repetitions as f64);
                        }
                        times.sort_by(f64::total_cmp);
                        println!(
                            "{workers},{units},{operation},{repetitions},7,{:.3},{:.3},{:.3}",
                            times[3], times[0], times[6]
                        );
                    }
                }
            });
    }
}
