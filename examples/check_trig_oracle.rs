use matrust::files;
use std::path::Path;
fn main() {
    let data = files::load_workspace(Path::new(
        "target/evidence/performance/data/trig-oracle.mat",
    ))
    .unwrap();
    let x = &data["x"];
    let mut total = 0;
    let mut maximum = 0_f64;
    for (name, actual) in [("sine", x.sin()), ("cosine", x.cos())] {
        assert_eq!(actual.len(), data[name].len());
        for ((&x, &expected), &actual) in x.data().iter().zip(data[name].data()).zip(actual.data())
        {
            if expected.is_nan() {
                assert!(actual.is_nan());
            } else {
                let error = (actual - expected).abs();
                maximum = maximum.max(error);
                assert!(
                    error <= 1e-15,
                    "{name}({x}): actual {actual}, MATLAB {expected}, error {error}"
                );
            }
            total += 1;
        }
    }
    println!(
        "Trigonometric reference cases: {total} values, maximum finite absolute error {maximum:e}"
    );
}
