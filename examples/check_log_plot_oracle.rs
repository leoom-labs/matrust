use matrust::{
    Engine, files,
    plot::{AxisScale, Figure},
};
use std::path::Path;
fn run(script: &str) -> Figure {
    let mut e = Engine::default();
    e.execute(script).unwrap();
    e.figure.unwrap()
}
fn main() {
    let data =
        files::load_workspace(Path::new("tests/fixtures/reference/log-plot-oracle.mat")).unwrap();
    let mut checked = 0;
    for (script, samples, scales, limits) in [
        (
            "semilogx([1 10 100],[2 20 200],'r--o');",
            "logx",
            "scales1",
            "",
        ),
        (
            "semilogx([1 10 100],[2 20 200]);hold on;semilogy([1 10],[2 3]);",
            "",
            "scalesheld",
            "",
        ),
        (
            "loglog([-100 -10 -1],[1 10 100]);axis([-100 -1 1 100]);",
            "negative",
            "scalesnegative",
            "limitsnegative",
        ),
        (
            "semilogx([-10 0 1 10 NaN 100],[1 2 3 4 5 6]);",
            "mixed",
            "",
            "",
        ),
        ("hold on;semilogx([1 10],[2 3]);", "", "scalespreheld", ""),
    ] {
        let f = run(script);
        if !samples.is_empty() {
            for (actual, &expected) in f.series[0]
                .points
                .iter()
                .flatten()
                .zip(data[samples].data())
            {
                assert!(actual == expected || actual.is_nan() && expected.is_nan());
                checked += 1;
            }
        }
        if !scales.is_empty() {
            for (actual, &expected) in [f.xscale, f.yscale].into_iter().zip(data[scales].data()) {
                assert_eq!((actual == AxisScale::Log) as u8 as f64, expected);
                checked += 1;
            }
        }
        if !limits.is_empty() {
            let (x, y) = f.bounds();
            assert_eq!([x[0], x[1], y[0], y[1]], data[limits].data());
            checked += 4;
        }
    }
    println!(
        "Logarithmic plotting reference cases: {checked} values/properties agree exactly (NaNs equivalent)"
    );
}
