//! Release benchmark: cargo run --release --example benchmark_linear_models -- 1
//! Repeat with N workers; the bounded QR kernels intentionally stay serial.
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
    pool.install(|| {
        let cases=[
            ("regress 2000x8", "rng(42);X=[ones(2000,1) randn(2000,8)];y=X*[1;1;2;3;4;5;6;7;8]+randn(2000,1);", "[b,bi,r,ri,s]=regress(y,X);",20),
            ("regress 2000x64", "rng(42);X=[ones(2000,1) randn(2000,64)];y=randn(2000,1);", "[b,bi,r,ri,s]=regress(y,X);",10),
            ("OLS 2000x8", "rng(42);X=randn(2000,8);y=1+X*[1;2;3;4;5;6;7;8]+randn(2000,1);", "m=fitlm(X,y);[p,ci]=predict(m,X);",20),
            ("OLS 20000x8", "rng(42);X=randn(20000,8);y=1+X*[1;2;3;4;5;6;7;8]+randn(20000,1);", "m=fitlm(X,y);[p,ci]=predict(m,X);",5),
            ("binomial 2000x4", "rng(42);X=randn(2000,4);y=double(rand(2000,1)<1./(1+exp(-X(:,1))));", "m=fitglm(X,y,'Distribution','binomial');p=predict(m,X);",10),
            ("poisson 2000x4", "rng(42);X=randn(2000,4);y=floor(abs(X(:,1)+2));", "m=fitglm(X,y,'Distribution','poisson');p=predict(m,X);",10),
            ("stepwise 500x4", "rng(42);X=randn(500,4);y=2+3*X(:,1)+2*X(:,2)+randn(500,1);", "m=stepwiselm(X,y);",5),
        ];
        println!("workers={workers}; max rows=1000000 terms=256 design=2000000 QR work=100000000; stepwise fits=1024");
        for (name,setup,code,reps) in cases {
            let mut e=Engine::default(); e.execute(setup).unwrap(); e.execute(code).unwrap();
            let start=Instant::now();
            for _ in 0..reps { e.execute(code).unwrap(); }
            println!("{name}: {:.3} ms/iteration ({reps} repetitions)",start.elapsed().as_secs_f64()*1000./reps as f64);
        }
    });
}
