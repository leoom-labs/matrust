use matrust::files;
use std::path::Path;
fn main() {
    let data = files::load_workspace(Path::new(
        "target/evidence/performance/data/matmul-oracle.mat",
    ))
    .unwrap();
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
            let mut count=0;let mut maximum=0_f64;
            for i in 1..=6 {
                let a=&data[&format!("a{i}")];let b=&data[&format!("b{i}")];let expected=&data[&format!("c{i}")];
                let actual=a.mul(b).unwrap();
                assert_eq!((actual.rows(),actual.cols()),(expected.rows(),expected.cols()));
                let tolerance=32.*f64::EPSILON*a.cols() as f64;
                for (&x,&y) in actual.data().iter().zip(expected.data()) {
                    let error=(x-y).abs();maximum=maximum.max(error);
                    assert!(error<=tolerance,"case {i}: {error} > {tolerance}");count+=1;
                }
            }
            println!("Matrix multiplication reference cases, {threads} workers: {count} values, max absolute error {maximum:e}");
        });
    }
}
