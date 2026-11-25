//! Headless release benchmark. Run with worker count 1, then 4:
//! bash tool/control-responses-cargo.sh run --release --no-default-features \
//!   --example benchmark_control_responses -- 1
//! Includes interpreter/output publication and Hessenberg reduction; no graphics.
use matrust::{Engine, initialize_worker_pool};
use std::{hint::black_box, time::Instant};
fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1".into())
        .parse()
        .unwrap();
    initialize_worker_pool(workers).unwrap();
    println!("workers,kind,states,inputs,outputs,samples,repetitions,median_ms");
    for (n, m, p, nf, reps) in [
        (2, 1, 1, 50, 20),
        (8, 2, 2, 100, 15),
        (32, 2, 2, 500, 10),
        (64, 4, 4, 1000, 7),
        (128, 2, 2, 1000, 5),
    ] {
        let mut e = Engine::default();
        e.execute(&format!("n={n}; A=-diag(1:n)+0.002*ones(n); B=reshape(sin(1:n*{m}),n,{m}); C=reshape(cos(1:n*{p}),{p},n); G=ss(A,B,C,zeros({p},{m})); w=logspace(-2,2,{nf});")).unwrap();
        e.execute(&format!("Gd=ss(eye(n)+0.001*A,B,C,zeros({p},{m}),0.01);"))
            .unwrap();
        e.execute(&format!(
            "H=ss(-eye(n),ones(n,{p}),ones({m},n),0); K=0.05*H;"
        ))
        .unwrap();
        for (kind, script) in [
            ("frequency", "Hw=freqresp(G,w);"),
            ("time", "[y,t]=step(G,linspace(0,10,1000));"),
            ("time_states", "[y,t,x]=step(G,linspace(0,10,1000));"),
            ("feedback", "R=feedback(G,K);"),
            ("series", "R=series(G,H);"),
            ("parallel", "R=parallel(G,G);"),
            ("append", "R=append(G,G);"),
            ("horizontal", "R=[G G];"),
            ("vertical", "R=[G;G];"),
            ("discrete", "[y,t]=step(Gd,linspace(0,9.99,1000));"),
            ("discrete_auto", "[y,t]=step(Gd);"),
        ] {
            e.execute(script).unwrap();
            let mut samples = Vec::new();
            for _ in 0..reps {
                let start = Instant::now();
                black_box(e.execute(script).unwrap());
                samples.push(start.elapsed().as_secs_f64() * 1000.);
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "{workers},{kind},{n},{m},{p},{},{reps},{:.6}",
                match kind {
                    "frequency" => nf,
                    "time" | "time_states" | "discrete" | "discrete_auto" =>
                        e.array("t").unwrap().rows(),
                    _ => 0,
                },
                samples[reps / 2]
            );
        }
    }
}
