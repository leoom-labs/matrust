//! Run with cargo run --release --no-default-features --example benchmark_control_design -- 1
//! and repeat with 4. Measures interpreter plus design, not isolated kernel throughput.
use matrust::Engine;
use std::time::Instant;
fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1".into())
        .parse()
        .unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap();
    println!("workers,path,states,inputs,repetitions,total_ms");
    pool.install(|| {
        for n in [2, 8, 32, 128] {
            let setup = format!(
                "A=-diag(1:{n})+diag(ones(1,{}),1)*0.1; Ad=A/{}; B=ones({n},2); Q=eye({n}); R=eye(2);",
                n - 1, n + 8
            );
            for (path, code) in [
                ("ctrb", "T=ctrb(A,B);"),
                ("obsv", "O=obsv(A,B');"),
                ("care", "[X,K,L]=care(A,B,Q,R);"),
                ("dare", "[X,K,L]=dare(Ad,B,Q,R);"),
            ] {
                let mut e = Engine::default();
                e.execute(&setup).unwrap();
                e.execute(code).unwrap();
                let repetitions = 5;
                let start = Instant::now();
                for _ in 0..repetitions {
                    e.execute(code).unwrap();
                }
                println!(
                    "{workers},{path},{n},2,{repetitions},{:.6}",
                    start.elapsed().as_secs_f64() * 1000.
                );
            }
        }
        for (path, setup, code, n) in [
            ("place", "A=[0 1;0 0]; B=[0;1];", "K=place(A,B,[-2 -3]);", 2),
            ("acker", "A=[0 1;0 0]; B=[0;1];", "K=acker(A,B,[-2 -3]);", 2),
            (
                "place_complex",
                "A=[1 2 0 0;3 4 0 0;0 0 1 2;0 0 3 4]; B=[1 0;0 1;0 1;1 0];",
                "K=place(A,B,[-1+2i -1-2i -3 -4]);",
                4,
            ),
            (
                "rlocus_fixed",
                "G=tf([1 2],[1 4 5 0]);",
                "[r,k]=rlocus(G,[0 .1 1 10 100]);",
                3,
            ),
            (
                "rlocus_auto",
                "G=tf([1 2],[1 4 5 0]);",
                "[r,k]=rlocus(G);",
                3,
            ),
        ] {
            let inputs = if path == "place_complex" { 2 } else { 1 };
            let mut e = Engine::default();
            e.execute(setup).unwrap();
            e.execute(code).unwrap();
            let repetitions = 5;
            let start = Instant::now();
            for _ in 0..repetitions {
                e.execute(code).unwrap();
            }
            println!(
                "{workers},{path},{n},{inputs},{repetitions},{:.6}",
                start.elapsed().as_secs_f64() * 1000.
            );
        }
        // Fix-round regressions: high-order rank-one placement and the
        // seeded mid-size Riccati cases. Setup is outside the timed section.
        for n in [10, 12] {
            let mut e = Engine::default();
            e.execute(&format!("A=diag(ones(1,{}),1); B=zeros({n},1); B({n})=1; p=-(1:{n});", n-1)).unwrap();
            let code = "K=place(A,B,p);";
            e.execute(code).unwrap();
            let repetitions = 5;
            let start = Instant::now();
            for _ in 0..repetitions {
                e.execute(code).unwrap();
            }
            println!(
                "{workers},place_chain,{n},1,{repetitions},{:.6}",
                start.elapsed().as_secs_f64() * 1000.
            );
        }
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/control/design.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            if !name.ends_with("_seed4_n60") {
                continue;
            }
            let (setup, call) = case["code"]
                .as_str()
                .unwrap()
                .rsplit_once("[X,K,L]=")
                .unwrap();
            let code = format!("[X,K,L]={call}");
            let mut e = Engine::default();
            e.execute(setup).unwrap();
            e.execute(&code).unwrap();
            let repetitions = 5;
            let start = Instant::now();
            for _ in 0..repetitions {
                e.execute(&code).unwrap();
            }
            println!(
                "{workers},{name},60,5,{repetitions},{:.6}",
                start.elapsed().as_secs_f64() * 1000.
            );
        }
    });
}
