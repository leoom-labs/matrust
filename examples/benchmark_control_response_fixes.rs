//! Fix-round release benchmark; run with 1, then 4 workers:
//! bash tool/control-responses-cargo.sh run --release --no-default-features \
//!   --example benchmark_control_response_fixes -- 1
//! Model setup is excluded; interpreter, response allocation and publication
//! are included. Each case has one warmup and 100 timed repetitions.
use matrust::{Engine, initialize_worker_pool};
use std::{hint::black_box, time::Instant};

fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1".into())
        .parse()
        .unwrap();
    initialize_worker_pool(workers).unwrap();
    println!("workers,case,states,inputs,outputs,samples,repetitions,median_ms");
    for (label, setup, script, n, m, p, samples) in [
        (
            "margin_high_gain_integrator",
            "G=tf(1000,[1 1 0]);",
            "[gm,pm,wg,wp]=margin(G);",
            2,
            1,
            1,
            "auto",
        ),
        (
            "margin_high_gain_first_order",
            "G=tf(100,[1 1]);",
            "[gm,pm,wg,wp]=margin(G);",
            1,
            1,
            1,
            "auto",
        ),
        (
            "margin_high_phase",
            "G=tf([1 3.0001],[1 3 3 1]);",
            "[gm,pm,wg,wp]=margin(G);",
            3,
            1,
            1,
            "auto",
        ),
        (
            "margin_discrete",
            "G=tf(1,[1 -0.5],0.1);",
            "[gm,pm,wg,wp]=margin(G);",
            1,
            1,
            1,
            "1000",
        ),
        (
            "step_near_zero",
            "T=[1 2 0;0 1 3;1 0 1]; A=T*diag([0 -1 -2])/T; G=ss(A,[1;0;0],[1 0 0],0);",
            "[y,t]=step(G);",
            3,
            1,
            1,
            "auto",
        ),
        (
            "step_undamped",
            "T=[1 2;3 4]; G=ss(T*[0 1;-1 0]/T,[0;1],[1 0],0);",
            "[y,t]=step(G);",
            2,
            1,
            1,
            "auto",
        ),
        (
            "step_near_undamped_discrete",
            "r=1-1e-12; a=r*[cos(0.1) sin(0.1);-sin(0.1) cos(0.1)]; G=ss(a,[0;1],[1 0],0,0.1);",
            "[y,t]=step(G);",
            2,
            1,
            1,
            "auto",
        ),
        (
            "freqresp_40_states",
            "G=ss(-eye(40),ones(40,1),ones(1,40),0); w=logspace(-2,2,1000);",
            "H=freqresp(G,w);",
            40,
            1,
            1,
            "1000",
        ),
        (
            "bode_mimo_plot",
            "G=ss(-diag(1:8),ones(8,4),ones(2,8),zeros(2,4)); w=logspace(-2,2,1000);",
            "bode(G,w);",
            8,
            4,
            2,
            "1000",
        ),
    ] {
        let mut e = Engine::default();
        e.execute(setup).unwrap();
        e.execute(script).unwrap();
        let mut elapsed = Vec::new();
        for _ in 0..100 {
            let start = Instant::now();
            black_box(e.execute(script).unwrap());
            elapsed.push(start.elapsed().as_secs_f64() * 1000.);
        }
        elapsed.sort_by(f64::total_cmp);
        let samples = if label.starts_with("margin_") {
            // Read the search-grid size from an untimed plot of the same model.
            e.execute("margin(G);").unwrap();
            e.figure
                .as_ref()
                .unwrap()
                .all_axes()
                .next()
                .unwrap()
                .1
                .series[0]
                .points
                .len()
                .to_string()
        } else if label.starts_with("step_") {
            e.array("t").unwrap().rows().to_string()
        } else {
            samples.to_owned()
        };
        println!(
            "{workers},{label},{n},{m},{p},{samples},100,{:.6}",
            elapsed[50]
        );
    }
}
