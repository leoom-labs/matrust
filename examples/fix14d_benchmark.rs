use matrust::{Engine, initialize_worker_pool};
use std::{hint::black_box, time::Instant};
fn main() {
    let workers: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    initialize_worker_pool(workers).unwrap();
    let mut e = Engine::default();
    e.execute("a=single(reshape(1:1048576,1024,1024)); b=int64(a); z=complex(double(a),double(a)); l=logical(mod(double(a),2));").unwrap();
    for (name, expression) in [
        ("sum-all", "sum(a,'all')"),
        ("sum-default", "sum(a,'default')"),
        ("mean", "mean(a,'all')"),
        ("range-int", "range(b,'all')"),
        ("range-dim3", "range(a,3)"),
        ("median", "median(l,[1 3])"),
        ("max-all", "max(z,[],'all')"),
        ("max-plain", "max(z)"),
        ("mode-nan", "mode(NaN)"),
    ] {
        let statement = format!("{expression};");
        e.execute(&statement).unwrap();
        let mut times = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            black_box(e.execute(&statement).unwrap());
            times.push(start.elapsed().as_secs_f64() * 1000.);
        }
        times.sort_by(f64::total_cmp);
        println!("{workers},{name},{:.3}", times[3]);
    }
}
