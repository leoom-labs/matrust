//! Release regression measurements; explicit one/four-worker caller pools.
//! `cargo run --release --example benchmark_xcorr_ode --offline [-- long]`
use matrust::{Engine, Matrix};
use std::{hint::black_box, time::Instant};

fn measure(e: &mut Engine, label: &str, workers: usize, n: usize, source: &str) {
    e.execute(source).unwrap();
    let mut times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        black_box(e.execute(black_box(source)).unwrap());
        times.push(start.elapsed().as_secs_f64() * 1000.);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{label},{workers},{n},5,{:.6},{:.6},{:.6}",
        times[2], times[0], times[4]
    );
}

fn main() {
    let long = std::env::args().any(|a| a == "long");
    println!("case,workers,size,repetitions,median_ms,min_ms,max_ms");
    for workers in [1, 4] {
        rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap().install(|| {
            let mut e = Engine::default();
            if long {
                let mut times = Vec::new();
                for _ in 0..3 {
                    let start = Instant::now();
                    e.execute("sol=ode45(@(t,y) y,[0 2250000],[0;0],odeset('RelTol',1e-12,'AbsTol',1e-14,'InitialStep',1,'MaxStep',1)); n=sol.stats.nfevals;").unwrap();
                    times.push(start.elapsed().as_secs_f64() * 1000.);
                }
                times.sort_by(f64::total_cmp);
                println!("ode45_long,{workers},{},3,{:.6},{:.6},{:.6}", 2. * e.variables["n"].data()[0], times[1], times[0], times[2]);
                return;
            }
            for n in [32, 256, 358, 360, 506, 508, 512, 1024, 12000, 100000, 441000] {
                let x = Matrix::from_column_vec(1, n, (0..n).map(|i| (i as f64 * 0.1).sin()).collect()).unwrap();
                e.variables.insert("x".into(), x);
                measure(&mut e, "xcorr_full", workers, n, "r=xcorr(x);");
                measure(&mut e, "xcorr_lag8", workers, n, "r=xcorr(x,8);");
                measure(&mut e, "xcorr_cross", workers, n, "r=xcorr(x,x);");
            }
            for solver in ["ode45", "ode23"] {
                measure(&mut e, solver, workers, 2, &format!("[t,y]={solver}(@(t,y) -y,[0 10],[1;2],odeset('RelTol',1e-9,'AbsTol',1e-11));"));
            }
        });
    }
}
