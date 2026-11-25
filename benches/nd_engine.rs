#[path = "nd/affinity.rs"]
mod affinity;
#[path = "nd/paired.rs"]
mod paired;
use matrust::Engine;
use std::{hint::black_box, time::Instant};

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--paired-server") {
        paired::serve(std::env::args().nth(2).unwrap().parse().unwrap());
        return;
    }
    let label = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "unspecified".into());
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(8);
    println!("revision={label},shape=1000x1000,workers={workers},repetitions=101");
    println!("operation,threads,median_ms,min_ms,max_ms");
    for threads in [1, workers] {
        let pool = rayon::ThreadPoolBuilder::new()
            .start_handler(affinity::bind_worker)
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            for (name, statement) in [
                ("A+B", "C=A+B;"),
                ("[m,i]=max(A+B)", "[m,i]=max(A+B);"),
                ("sum(A)", "C=sum(A);"),
                ("A(:,500)", "C=A(:,500);"),
            ] {
                let mut engine = Engine::default();
                engine
                    .execute("A=reshape(1:1000000,1000,1000); B=ones(1000,1000);")
                    .unwrap();
                // Batch the microsecond indexing path so formatting/timer noise
                // does not turn sub-nanosecond changes into a 10% rounded delta.
                let batch = if name == "A(:,500)" { 1000 } else { 1 };
                let mut samples = Vec::with_capacity(101);
                for _ in 0..10 {
                    black_box(engine.execute(statement).unwrap());
                }
                for _ in 0..101 {
                    let start = Instant::now();
                    for _ in 0..batch {
                        black_box(engine.execute(black_box(statement)).unwrap());
                    }
                    samples.push(start.elapsed().as_secs_f64() * 1000. / batch as f64);
                }
                samples.sort_by(f64::total_cmp);
                println!(
                    "{name},{threads},{:.6},{:.6},{:.6}",
                    samples[50], samples[0], samples[100]
                );
            }
        });
    }
}
