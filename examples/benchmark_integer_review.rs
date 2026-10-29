//! Release measurements for the integer review fixes; one process per pool size.
//! cargo run --release --features desktop --example benchmark_integer_review -- 1
//! Repeat with 4. Reports median of five samples after one warmup, no console echo.
use matrust::Engine;
use std::time::Instant;

fn measure(engine: &mut Engine, label: &str, source: &str) {
    if std::env::var("MATRUST_BENCH_FILTER").is_ok_and(|filter| !label.contains(&filter)) {
        return;
    }
    engine.execute(source).unwrap();
    let mut elapsed = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        engine.execute(source).unwrap();
        elapsed.push(start.elapsed().as_secs_f64() * 1000.);
    }
    elapsed.sort_by(f64::total_cmp);
    println!(
        "{label}: median_ms={:.3} min_ms={:.3} max_ms={:.3} repetitions=5",
        elapsed[2], elapsed[0], elapsed[4]
    );
}

fn main() {
    let threads = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1".into())
        .parse()
        .unwrap();
    matrust::initialize_worker_pool(threads).unwrap();
    println!("integer review benchmark; threads={threads}; release; one warmup per operation");
    for side in [32, 512, 1024] {
        let n = side * side;
        let mut e = Engine::default();
        // Values are deterministic and genuinely exceed 2^53, with repeated keys.
        e.execute(&format!(
            "a=int64(mod((1:{n})*7919,65521))+int64(9007199254740993); b=reshape(a,{side},{side});"
        ))
        .unwrap();
        measure(&mut e, &format!("sort int64 1x{n}"), "x=sort(a);");
        measure(&mut e, &format!("unique int64 1x{n}"), "x=unique(a);");
        measure(&mut e, &format!("max int64 {side}x{side}"), "x=max(b);");
        measure(
            &mut e,
            &format!("cumsum int64 {side}x{side}"),
            "x=cumsum(b);",
        );
        measure(&mut e, &format!("isequal int64 1x{n}"), "x=isequal(a,a);");
        measure(&mut e, &format!("assignment int64 1x{n}"), "x=a; x(2)=7;");
        measure(&mut e, &format!("index int64 1x{n}"), "x=a(:);");
        measure(&mut e, &format!("add int64 1x{n}"), "x=a+1;");
        measure(&mut e, &format!("multiply int64 1x{n}"), "x=a.*a;");
        measure(
            &mut e,
            &format!("cast and add single {side}x{side}"),
            "x=single(b)+single(1);",
        );
        e.execute("clear;").unwrap();
    }
    for rows in [1024, 262144, 524288] {
        let mut e = Engine::default();
        e.execute(&format!(
            "a=int64(mod((1:{rows})'*7919,65521))+int64(9007199254740993); a=[a -a];"
        ))
        .unwrap();
        measure(
            &mut e,
            &format!("sortrows int64 {rows}x2"),
            "x=sortrows(a);",
        );
    }
    for side in [32, 256] {
        let mut e = Engine::default();
        e.execute(&format!("a=single(eye({side})+ones({side})/{side});"))
            .unwrap();
        measure(&mut e, &format!("det single {side}x{side}"), "x=det(a);");
        measure(
            &mut e,
            &format!("matrix power single {side}x{side}"),
            "x=a^2;",
        );
    }
}
