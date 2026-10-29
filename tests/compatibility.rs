use matrust::{Engine, Matrix};
fn value(source: &str) -> Matrix {
    let mut e = Engine::default();
    e.execute(source).unwrap();
    e.variables["ans"].clone()
}
fn close(a: &[f64], b: &[f64]) {
    assert_eq!(a.len(), b.len());
    for (&x, &y) in a.iter().zip(b) {
        if (x.is_nan() && y.is_nan()) || x == y {
            continue;
        }
        assert!((x - y).abs() <= 1e-11 * y.abs().max(1.0), "{x} != {y}");
    }
}
#[test]
fn matrix_and_elementwise_products_differ() {
    close(value("[1 2;3 4]*[1 2;3 4]").data(), &[7., 15., 10., 22.]);
    close(value("[1 2;3 4].*[1 2;3 4]").data(), &[1., 9., 4., 16.]);
}
#[test]
fn solve_has_small_residual() {
    let mut e = Engine::default();
    e.execute(include_str!("../examples/linear_system.m"))
        .unwrap();
    assert!(e.variables["residual"].as_scalar().unwrap() < 1e-12);
}
#[test]
fn indexing_is_one_based_and_column_major() {
    close(value("A=[1 2;3 4]; A(:)").data(), &[1., 3., 2., 4.]);
    close(value("A=[1 2;3 4]; A(:,end)").data(), &[2., 4.]);
    close(value("A=[1 2;3 4]; A(2:3)").data(), &[3., 2.]);
    assert_eq!(
        (
            value("A=[1;2;3]; A([1 2])").rows(),
            value("A=[1;2;3]; A([1 2])").cols()
        ),
        (2, 1)
    );
}
#[test]
fn matlab_precedence_and_spacing() {
    close(value("-2^2").data(), &[-4.]);
    close(value("2^3^2").data(), &[64.]);
    close(value("[1 -2]").data(), &[1., -2.]);
    close(value("[1 - 2]").data(), &[-1.]);
    close(value("2^-2").data(), &[0.25]);
}
#[test]
fn reductions_follow_first_nonsingleton_dimension() {
    close(value("sum([1 2;3 4])").data(), &[4., 6.]);
    close(value("sum([1 2 3])").data(), &[6.]);
    close(value("mean([1 2;3 4])").data(), &[2., 3.]);
}
#[test]
fn concatenation_and_transpose() {
    close(
        value("A=[1 2;3 4]; [A A]'").data(),
        &[1., 2., 1., 2., 3., 4., 3., 4.],
    );
}
#[test]
fn malformed_and_oversize_inputs_return_errors() {
    for s in [
        "zeros(1e12)",
        "ones(-0.5)",
        "[1 2;3]",
        "[1 2]*[1 2]",
        "A=[1 2]; A(0)",
        "A=[1 2]; A(3)",
        "A=[1 2]; A(1.5)",
        "1:0:3",
        "(1+2",
        "ones()",
    ] {
        assert!(Engine::default().execute(s).is_err(), "accepted {s}");
    }
}
#[test]
fn quiet_statements_and_recovery() {
    let mut e = Engine::default();
    assert!(e.execute("A=eye(2);").unwrap().output.is_empty());
    assert!(e.execute("ones(-0.5)").is_err());
    assert!(e.execute("A*2").is_ok());
}
#[test]
fn empty_and_descending_ranges() {
    assert!(value("3:1").is_empty());
    close(value("3:-1:1").data(), &[3., 2., 1.]);
    close(value("linspace(0,4,1)").data(), &[4.]);
    assert!(value("linspace(0,4,0)").is_empty());
}
#[test]
fn plotting_escapes_labels_and_breaks_nonfinite_lines() {
    let mut e = Engine::default();
    e.execute("plot([1 2 3], [1 NaN 3]); title('<script>')")
        .unwrap();
    let svg = e.figure.unwrap().svg();
    assert!(svg.contains("&lt;script&gt;"));
    assert!(!svg.contains("<script>"));
    assert!(!svg.contains("NaN"));
}
#[test]
fn cancellation_is_reported() {
    let mut e = Engine::default();
    e.cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(e.execute("ones(4)").unwrap_err().contains("cancelled"));
}
#[test]
fn expression_complexity_is_bounded() {
    assert!(
        Engine::default()
            .execute(&vec!["1"; 2000].join("+"))
            .is_err()
    );
    assert!(
        Engine::default()
            .execute(&format!("{}1{}", "(".repeat(200), ")".repeat(200)))
            .is_err()
    );
}
#[test]
fn parallel_gemm_agrees_with_serial() {
    let n = 224;
    let a = Matrix::from_column_slice(
        n,
        n,
        &(0..n * n)
            .map(|i| (i % 19) as f64 / 19.0)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let serial = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(|| a.mul(&a).unwrap());
    let parallel = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap()
        .install(|| a.mul(&a).unwrap());
    close(serial.data(), parallel.data());
}
#[test]
fn reference_cases() {
    let fixture = include_str!("../tests/fixtures/reference/reference.tsv");
    let mut count = 0;
    for line in fixture.lines().filter(|s| !s.trim().is_empty()) {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 4);
        let m = value(fields[0]);
        assert_eq!(
            m.rows(),
            fields[1].parse::<usize>().unwrap(),
            "{}",
            fields[0]
        );
        assert_eq!(
            m.cols(),
            fields[2].parse::<usize>().unwrap(),
            "{}",
            fields[0]
        );
        let expected: Vec<f64> = fields[3]
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().unwrap())
            .collect();
        close(m.data(), &expected);
        count += 1;
    }
    assert_eq!(count, 26, "The MATLAB oracle must contain all 26 cases");
}
