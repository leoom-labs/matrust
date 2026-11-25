use matrust::Engine;
use std::{hint::black_box, time::Instant};
fn main() {
    println!("50000 cellstr elements, 9 warmups, 51 samples; sequential text paths");
    for workers in [1, 4] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                for (name, code) in [
                    ("upper", "R=upper(C);"),
                    ("strrep", "R=strrep(C,'abc','XY');"),
                    ("strcat", "R=strcat(C,'z');"),
                ] {
                    let mut e = Engine::default();
                    e.execute("C=repmat({'abc def'},1,50000);").unwrap();
                    for _ in 0..9 {
                        black_box(e.execute(code).unwrap());
                    }
                    let mut samples = Vec::new();
                    for _ in 0..51 {
                        let start = Instant::now();
                        black_box(e.execute(code).unwrap());
                        samples.push(start.elapsed().as_secs_f64() * 1000.);
                    }
                    samples.sort_by(f64::total_cmp);
                    println!(
                        "{name},workers={workers},median_ms={:.6},min_ms={:.6},max_ms={:.6}",
                        samples[25], samples[0], samples[50]
                    );
                }
            });
    }
}
