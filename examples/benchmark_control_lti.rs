//! End-to-end LTI release benchmark. No UI and no additional kernel thread pool.
//! Run: cargo run --release --no-default-features --example benchmark_control_lti
use matrust::Engine;
use std::{hint::black_box, time::Instant};

fn measure(label: &str, setup: &str, script: &str, repeats: usize) {
    let mut e = Engine::default();
    e.execute(setup).unwrap();
    for _ in 0..3 {
        e.execute(script).unwrap();
    }
    let start = Instant::now();
    for _ in 0..repeats {
        black_box(e.execute(script).unwrap());
    }
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "{label},repetitions={repeats},total_ms={:.3},us_per_op={:.3}",
        elapsed * 1000.,
        elapsed * 1e6 / repeats as f64
    );
}
fn measure_mimo() {
    let small = "S=ss([0 1;-2 -3],eye(2),eye(2),zeros(2)); F=tf(S); Z=zpk(S);";
    let medium = concat!(
        "A=-diag(1:12); B=reshape(sin(1:96),12,8); C=reshape(cos(1:72),6,12); ",
        "S=ss(A,B,C,zeros(6,8)); F=tf(S); Z=zpk(S);",
    );
    for (name, setup, script, repeats) in [
        (
            "siso_tf_to_ss_order2",
            "F=tf([1 3],[1 2 5]);",
            "R=ss(F);",
            3000,
        ),
        ("mimo_ss_to_tf_2x2_order2", small, "R=tf(S);", 3000),
        ("mimo_tf_to_ss_2x2_order2", small, "R=ss(F);", 3000),
        ("mimo_ss_to_zpk_2x2_order2", small, "R=zpk(S);", 3000),
        ("mimo_zpk_to_tf_2x2_order2", small, "R=tf(Z);", 3000),
        ("mimo_zpk_to_ss_2x2_order2", small, "R=ss(Z);", 3000),
        ("mimo_ss_to_tf_6x8_order12", medium, "R=tf(S);", 300),
        ("mimo_tf_to_ss_6x8_order12", medium, "R=ss(F);", 50),
        ("mimo_ss_to_zpk_6x8_order12", medium, "R=zpk(S);", 50),
        (
            "mimo_channel_index_6x8",
            medium,
            "R=F([6 2 1 5],[8 7 4 3]);",
            3000,
        ),
        (
            "mimo_channel_assignment_6x8",
            medium,
            "R=F; R(5,7)=Z(6,8);",
            3000,
        ),
        ("mimo_evalfr_6x8_order12", medium, "R=evalfr(F,2+3i);", 3000),
        ("mimo_minreal_tf_6x8_order12", medium, "R=minreal(F);", 50),
    ] {
        measure(name, setup, script, repeats);
    }
    measure(
        "mimo_minreal_ss_order32_to_16",
        "S=ss(-diag(1:32),[ones(16,2);zeros(16,2)],ones(2,32),zeros(2));",
        "R=minreal(S);",
        300,
    );
}

fn main() {
    let mimo = std::env::args().any(|arg| arg == "--mimo");
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            println!("workers={workers}");
            if mimo {
                measure_mimo();
                return;
            }
            measure(
                "feedback_tf_order2",
                "G=tf([1 3],[1 2 5]); H=tf(2,[1 3 2]);",
                "R=feedback(G,H);",
                2000,
            );
            measure(
                "ss_to_tf_order2",
                "S=ss([0 1;-2 -3],[0;1],[1 0],0);",
                "R=tf(S);",
                2000,
            );
            measure("tf_to_ss_order2", "G=tf([1 3],[1 2 5]);", "R=ss(G);", 2000);
            let blocks = "S=ss(tf([1 3],[1 2 5])); H=ss(tf(2,[1 3 2]));";
            measure("feedback_ss_order2", blocks, "R=feedback(S,H);", 2000);
            measure("series_ss_order2", blocks, "R=S*H;", 2000);
            measure(
                "inverse_ss_order2",
                "S=ss(tf([1 3 8],[1 2 5]));",
                "R=inv(S);",
                2000,
            );
            measure(
                "minreal_tf_order2",
                "G=tf([1 3 2],[1 5 4]);",
                "R=minreal(G);",
                2000,
            );
            measure(
                "minreal_zpk_order33",
                "Z=zpk(-(1:32),[-(1:32) -40],1);",
                "R=minreal(Z,0);",
                200,
            );
            measure(
                "minreal_tf_split_real_order3",
                "G=tf([1 1],[1 3 3 1]);",
                "R=minreal(G,1e-4);",
                2000,
            );
            measure(
                "minreal_zpk_near_real_order2",
                "G=zpk(-1,[-1+1e-9i;-1-1e-9i],1);",
                "R=minreal(G);",
                2000,
            );
            measure(
                "zero_gain_zpk_to_ss",
                "G=zpk([1 2],[],0);",
                "R=ss(G);",
                2000,
            );
            measure(
                "damp_discrete_order2",
                "G=zpk([],[-0.5+0.2i;-0.5-0.2i],1,0.1);",
                "[w,z,p]=damp(G);",
                2000,
            );
            measure(
                "zpk_product_order32",
                "Z=zpk(-(1:32),-(33:64),1);",
                "R=Z*Z;",
                200,
            );
            let delayed = "G=tf(2,[3 3],'InputDelay',0.375); T=0:0.01:5; U=sin(T);";
            measure("delayed_step_501", delayed, "Y=step(G,T);", 200);
            measure("delayed_foh_501", delayed, "Y=lsim(G,U,T,'foh');", 200);
            measure(
                "delayed_bode_1000",
                delayed,
                "[M,P]=bode(G,logspace(-2,2,1000));",
                200,
            );
            measure("delayed_margin", delayed, "[Gm,Pm]=margin(G);", 100);
            measure(
                "margin_delayed_two_pole",
                "G=tf(5,[1 201 200],'InputDelay',2);",
                "[Gm,Pm]=margin(G);",
                100,
            );
            measure(
                "margin_delayed_integrator",
                "G=tf(1000,[1 100 0],'InputDelay',10);",
                "[Gm,Pm]=margin(G);",
                100,
            );
            measure(
                "lqr_unspecified_sample",
                "S=ss(0.5,1,1,0,-1);",
                "K=lqr(S,1,1);",
                100,
            );
            measure("delayed_c2d", delayed, "D=c2d(G,0.125);", 1000);
            measure("pade_order_3", delayed, "P=pade(G,3);", 1000);
            measure(
                "parallel_delay_step_501",
                "P=parallel(tf(1,[1 1],'InputDelay',0.375),tf(2,[1 2])); T=0:0.01:5;",
                "Y=step(P,T);",
                200,
            );
            for n in [4, 32, 96] {
                let setup =
                    format!("A=-diag(1:{n}); B=ones({n},2); C=ones(2,{n}); S=ss(A,B,C,zeros(2));");
                for method in ["zoh", "foh", "tustin"] {
                    measure(
                        &format!("c2d_{method}_states{n}_ports2"),
                        &setup,
                        &format!("R=c2d(S,0.02,'{method}');"),
                        if n == 96 { 10 } else { 50 },
                    );
                }
                measure(
                    &format!("assignment_states{n}_ports2"),
                    &setup,
                    "R=S;",
                    5000,
                );
                for method in ["zoh", "tustin"] {
                    let sampled = format!("{setup} Q=c2d(S,0.02,'{method}');");
                    measure(
                        &format!("d2c_{method}_states{n}_ports2"),
                        &sampled,
                        &format!("R=d2c(Q,'{method}');"),
                        if n == 96 { 5 } else { 20 },
                    );
                }
            }
        });
    }
}
