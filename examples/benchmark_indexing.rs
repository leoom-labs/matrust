//! End-to-end indexing benchmark. Build/copy this binary before changing the engine.
use matrust::{Engine, Matrix};
use std::{hint::black_box, io::Write, time::Instant};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let profile = args.get(1).is_some_and(|s| s == "--profile");
    let cases = [
        ("linear_all", "B=A(:);"),
        ("matrix_all", "B=A(:,:);"),
        ("columns", "B=A(:,2:7);"),
        ("row_stride", "B=A(1:2:end,:);"),
        ("reverse", "B=A(end:-1:1);"),
        ("explicit", "B=A(I,:);"),
        ("tiny_slice", "B=A(2:9,3);"),
    ];
    println!("case,elements,threads,repetitions,median_ms,min_ms,max_ms");
    for threads in if profile { vec![1] } else { vec![1, 8] } {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                for elements in if profile {
                    vec![8 * 1024 * 1024]
                } else {
                    vec![1024, 1024 * 1024, 8 * 1024 * 1024]
                } {
                    let rows = elements / 8;
                    let mut engine = Engine::default();
                    engine.variables.insert(
                        "A".into(),
                        Matrix::from_column_vec(rows, 8, (0..elements).map(|i| i as f64).collect())
                            .unwrap(),
                    );
                    engine.variables.insert(
                        "I".into(),
                        Matrix::from_column_vec(
                            1,
                            rows / 2,
                            (0..rows / 2).map(|i| (rows - 2 * i) as f64).collect(),
                        )
                        .unwrap(),
                    );
                    for (name, script) in cases {
                        if profile && args.get(2).is_none_or(|s| s != name) {
                            continue;
                        }
                        engine.execute(script).unwrap();
                        let output = &engine.variables["B"];
                        let expected_len = match name {
                            "columns" => rows * 6,
                            "row_stride" | "explicit" => elements / 2,
                            "tiny_slice" => 8,
                            _ => elements,
                        };
                        assert_eq!(output.len(), expected_len);
                        for (i, &v) in output.data().iter().enumerate() {
                            let expected = match name {
                                "columns" => i + rows,
                                "row_stride" => (i / (rows / 2)) * rows + (i % (rows / 2)) * 2,
                                "reverse" => elements - 1 - i,
                                "explicit" => {
                                    (i / (rows / 2)) * rows + rows - 1 - 2 * (i % (rows / 2))
                                }
                                "tiny_slice" => rows * 2 + 1 + i,
                                _ => i,
                            };
                            assert_eq!(v, expected as f64, "{name} value {i}");
                        }
                        if profile {
                            println!("READY {name} {} {}", output.rows(), output.cols());
                            std::io::stdout().flush().unwrap();
                            let mut line = String::new();
                            std::io::stdin().read_line(&mut line).unwrap();
                            black_box(&engine);
                            return;
                        }
                        for _ in 0..3 {
                            black_box(engine.execute(script).unwrap());
                        }
                        let repeats = 30;
                        let batch = if elements == 1024 || name == "tiny_slice" {
                            100
                        } else {
                            1
                        };
                        let mut times = Vec::new();
                        for _ in 0..repeats {
                            let start = Instant::now();
                            for _ in 0..batch {
                                black_box(engine.execute(black_box(script)).unwrap());
                            }
                            times.push(start.elapsed().as_secs_f64() * 1000. / batch as f64);
                        }
                        times.sort_by(f64::total_cmp);
                        println!(
                            "{name},{elements},{threads},{repeats},{:.6},{:.6},{:.6}",
                            (times[14] + times[15]) * 0.5,
                            times[0],
                            times[29]
                        );
                        engine.variables.remove("B");
                    }
                }
            });
    }
}
