use matrust::{Engine, Matrix};
use std::{hint::black_box, io::Write};

fn main() {
    let case = std::env::args()
        .nth(1)
        .expect("case: unique_cell, shared_cell, column, fill, growth");
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(|| {
            let rows = 1048576;
            let mut engine = Engine::default();
            engine.variables.insert(
                "A".into(),
                Matrix::from_column_vec(rows, 8, (0..rows * 8).map(|i| i as f64).collect())
                    .unwrap(),
            );
            if case == "shared_cell" {
                engine
                    .variables
                    .insert("B".into(), engine.variables["A"].clone());
            }
            let script = match case.as_str() {
                "unique_cell" | "shared_cell" => "A(2,3)=7;",
                "column" => "A(:,3)=7;",
                "fill" => "A(:)=7;",
                "growth" => "A(:,9)=7;",
                _ => panic!("unknown case"),
            };
            engine.execute(script).unwrap();
            let a = &engine.variables["A"];
            for (i, &x) in a.data().iter().enumerate() {
                let changed = match case.as_str() {
                    "unique_cell" | "shared_cell" => i == 2 * rows + 1,
                    "column" => i / rows == 2,
                    "fill" => true,
                    "growth" => i / rows == 8,
                    _ => unreachable!(),
                };
                assert_eq!(x, if changed { 7. } else { i as f64 });
            }
            println!("READY {case} {} {}", a.rows(), a.cols());
            std::io::stdout().flush().unwrap();
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).unwrap();
            black_box(&engine);
        });
}
