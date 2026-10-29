use matrust::{Engine, Matrix};
use std::{hint::black_box, time::Instant};
fn main() {
    println!("case,elements,threads,repetitions,median_ms");
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                for len in [131072, 262144, 524288, 1048576] {
                    for (name, script) in [("shared_cell", "A(2)=7;"), ("fill", "A(:)=7;")] {
                        let mut engine = Engine::default();
                        engine
                            .variables
                            .insert("A".into(), Matrix::filled(len / 8, 8, 0.).unwrap());
                        let mut times = Vec::new();
                        for i in 0..33 {
                            if name == "shared_cell" {
                                engine
                                    .variables
                                    .insert("B".into(), engine.variables["A"].clone());
                            }
                            let start = Instant::now();
                            black_box(engine.execute(script).unwrap());
                            if i >= 3 {
                                times.push(start.elapsed().as_secs_f64() * 1000.);
                            }
                            assert_eq!(engine.variables["A"].data()[1], 7.);
                        }
                        times.sort_by(f64::total_cmp);
                        println!(
                            "{name},{len},{threads},30,{:.6}",
                            (times[14] + times[15]) * 0.5
                        );
                    }
                }
            });
    }
}
