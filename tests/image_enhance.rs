//! histeq and edge. Expected values are worked by hand from the documented
//! rules (see src/engine/image_morph/enhance.rs): GNU Octave's image package
//! implements other algorithms for both (histeq returns the plain cumulative
//! histogram, edge another thinning), so it is no oracle here.
use matrust::Engine;

fn values(e: &mut Engine, expression: &str) -> Vec<f64> {
    e.execute(&format!("zz__ = double({expression});")).unwrap();
    e.variables["zz__"].data().to_vec()
}
fn run(source: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(source).unwrap();
    e
}
fn error(source: &str) -> String {
    Engine::default().execute(source).unwrap_err().to_string()
}

#[test]
fn histeq_spreads_distinct_levels_over_64_output_levels() {
    // Eight distinct pixels: pixel m (1..8) reaches output level 8m - 1 of 0..63.
    let mut e = run("J = histeq(uint8([0 10 20 30; 40 50 60 70]));");
    assert_eq!(values(&mut e, "isa(J, 'uint8')"), [1.]);
    let expected: Vec<f64> = [1., 5., 2., 6., 3., 7., 4., 8.]
        .iter()
        .map(|m: &f64| ((8. * m - 1.) / 63. * 255.).round())
        .collect();
    assert_eq!(values(&mut e, "J"), expected);
}

#[test]
fn histeq_flattens_a_ramp_and_returns_the_transform() {
    let mut e = run("[J, T] = histeq(uint8(repmat(0:99, 100, 1)), 4);");
    assert_eq!(values(&mut e, "imhist(J, 4)'"), [2500.; 4]);
    assert_eq!(values(&mut e, "size(T)"), [1., 256.]);
    // Values 0..24 go to level 0, 25..49 to 1/3, and so on; T is monotonic.
    assert_eq!(
        values(&mut e, "T([1 25 26 50 51 100])"),
        [0., 0., 1. / 3., 1. / 3., 2. / 3., 1.]
    );
    assert_eq!(values(&mut e, "all(diff(T) >= 0)"), [1.]);
}

#[test]
fn histeq_matches_a_target_histogram_for_doubles() {
    // n = 4 on eight pixels: cumulative counts 1 2 3 5 6 7 8 against 2 4 6 8.
    let mut e = run("J = histeq([0.1 0.2 0.3 0.9; 0.5 0.5 0.6 0.7], 4);");
    assert_eq!(values(&mut e, "J * 3"), [0., 1., 0., 1., 1., 2., 3., 3.]);
    // A target histogram puts most pixels in the bright half.
    let mut e = run("J = histeq(repmat(linspace(0, 1, 100), 10, 1), [1 3]);");
    assert_eq!(values(&mut e, "[nnz(J == 0) nnz(J == 1)]"), [250., 750.]);
    assert!(error("histeq(1, 1)").contains("n must be an integer"));
    assert!(error("histeq(1, [0 0])").contains("not all zero"));
    assert!(error("histeq(ones(2, 2, 2))").contains("2-D"));
}

#[test]
fn gradient_edges_are_one_pixel_wide_on_the_dark_side() {
    let mut e = run("A = zeros(20); A(:, 11:20) = 1;");
    for method in ["sobel", "prewitt", "roberts"] {
        e.execute(&format!("[bw, t] = edge(A, '{method}');"))
            .unwrap();
        assert_eq!(
            values(&mut e, "class(bw) == 'logical'"),
            [1.; 7],
            "{method}"
        );
        assert_eq!(values(&mut e, "find(any(bw, 1))"), [10.], "{method}");
        assert_eq!(values(&mut e, "nnz(bw)"), [20.], "{method}");
        // Squared gradient 0.25 on 40 of 400 pixels: sqrt(4 * 0.025).
        let t = values(&mut e, "t")[0];
        assert!((t - 0.1f64.sqrt()).abs() < 1e-12, "{method} {t}");
    }
    // Direction filters: a vertical step has no horizontal edges.
    assert_eq!(
        values(&mut e, "nnz(edge(A, 'sobel', [], 'horizontal'))"),
        [0.]
    );
    assert_eq!(
        values(&mut e, "nnz(edge(A, 'sobel', [], 'vertical'))"),
        [20.]
    );
    assert_eq!(
        values(&mut e, "find(any(edge(A, 'sobel', 0.1, 'nothinning'), 1))"),
        [10., 11.]
    );
    // Integer images are scaled to [0, 1] first.
    assert_eq!(
        values(&mut e, "isequal(edge(uint8(255 * A)), edge(A))"),
        [1.]
    );
}

#[test]
fn canny_finds_thin_closed_edges() {
    let mut e = run("A = zeros(20); A(:, 11:20) = 1; [bw, t] = edge(A, 'canny');");
    assert_eq!(values(&mut e, "find(any(bw, 1))"), [10.]);
    let t = values(&mut e, "t");
    assert!((t[0] - 0.4 * t[1]).abs() < 1e-12 && t[1] > 0. && t[1] < 1.);
    // A disk of radius 10 has one ring of edge pixels, every one beside the boundary.
    e.execute(
        "[x, y] = meshgrid(1:32); D = (x - 16.5).^2 + (y - 16.5).^2 < 100; \
         bw = edge(double(D), 'canny'); r = sqrt((x(bw) - 16.5).^2 + (y(bw) - 16.5).^2);",
    )
    .unwrap();
    let n = values(&mut e, "nnz(bw)")[0];
    assert!((50. ..=90.).contains(&n), "{n}");
    assert_eq!(values(&mut e, "all(abs(r - 10) < 1.5)"), [1.]);
    // Explicit thresholds come back as given; a flat image has no edges.
    e.execute("[bw, t] = edge(double(D), 'canny', [0.05 0.2], 2);")
        .unwrap();
    assert_eq!(values(&mut e, "t"), [0.05, 0.2]);
    assert_eq!(values(&mut e, "nnz(edge(ones(8), 'canny'))"), [0.]);
}

#[test]
fn edge_reports_unsupported_methods_and_bad_arguments() {
    assert!(error("edge(magic(4), 'log')").contains("not supported yet"));
    assert!(error("edge(magic(4), 'foo')").contains("unknown method"));
    assert!(error("edge(magic(4), 'canny', [0.5 0.2])").contains("low < high"));
    assert!(error("edge(magic(4), 'sobel', -1)").contains("nonnegative"));
    assert!(error("edge(magic(4), 'canny', [], 0)").contains("sigma"));
    assert!(error("edge(magic(4), 'sobel', [], 'diagonal')").contains("unsupported option"));
}
