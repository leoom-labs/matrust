use matrust::{Matrix, files};
use std::path::Path;
type Operation = (&'static str, fn(&Matrix) -> Matrix);
fn main() {
    let data = files::load_workspace(Path::new(
        "target/evidence/performance/data/elementwise-oracle.mat",
    ))
    .unwrap();
    let ops: [Operation; 8] = [
        ("tan", Matrix::tan),
        ("exp", Matrix::exp),
        ("log", Matrix::ln),
        ("sqrt", Matrix::sqrt),
        ("abs", Matrix::abs),
        ("floor", Matrix::floor),
        ("ceil", Matrix::ceil),
        ("round", Matrix::round),
    ];
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let mut total = 0;
                for (name, kernel) in ops {
                    let x = &data[&format!("x{name}")];
                    let expected = &data[&format!("y{name}")];
                    let actual = kernel(x);
                    assert_eq!(
                        (actual.rows(), actual.cols()),
                        (expected.rows(), expected.cols())
                    );
                    let mut maximum_scaled = 0_f64;
                    for ((&x, &a), &e) in x.data().iter().zip(actual.data()).zip(expected.data()) {
                        if e.is_nan() {
                            assert!(a.is_nan(), "{name}({x}): expected NaN");
                        } else if e == 0. || !e.is_finite() {
                            assert_eq!(a, e, "{name}({x})");
                        } else {
                            let scale = if name == "log" {
                                e.abs().max(1.)
                            } else {
                                e.abs().max(f64::MIN_POSITIVE)
                            };
                            let error = (a - e).abs() / scale;
                            maximum_scaled = maximum_scaled.max(error);
                            assert!(
                                error <= 2e-15,
                                "{name}({x}): actual={a}, MATLAB={e}, scaled_error={error}"
                            );
                        }
                        total += 1;
                    }
                    println!(
                        "{name}: {} values, {threads} workers, max scaled error {maximum_scaled:e}",
                        actual.len()
                    );
                }
                println!("Elementwise reference cases PASS: {total} values with {threads} workers");
            });
    }
}
