//! count, matlab.lang.makeValidName, lsqminnorm, givens, xcov and zplane.
//! References: givens from GNU Octave 9 (givens(-3, 4) = [0.6 -0.8; 0.8 0.6]);
//! xcov from NumPy (np.correlate of the mean-removed sequences); lsqminnorm by
//! hand (the minimum-norm solution of a rank-1 system lies along its row
//! space); count and makeValidName from their documented rules.
use matrust::Engine;

fn values(e: &mut Engine, expression: &str) -> Vec<f64> {
    e.execute(&format!("zz__ = double({expression});")).unwrap();
    e.variables["zz__"].data().to_vec()
}
fn error(source: &str) -> String {
    Engine::default().execute(source).unwrap_err().to_string()
}
fn close(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len(), "{actual:?}");
    for (a, b) in actual.iter().zip(expected) {
        assert!((a - b).abs() < 1e-12, "{actual:?} vs {expected:?}");
    }
}

#[test]
fn quick_names_exist() {
    let mut e = Engine::default();
    for name in [
        "count",
        "matlab.lang.makeValidName",
        "lsqminnorm",
        "givens",
        "xcov",
        "zplane",
    ] {
        assert!(
            values(&mut e, &format!("exist('{name}')"))[0] > 0.,
            "{name}"
        );
    }
}

#[test]
fn count_matches_each_segment_once() {
    let mut e = Engine::default();
    assert_eq!(
        values(&mut e, "count(\"the cat sat on the mat\", \"at\")"),
        [3.]
    );
    // Non-overlapping: 'aaaa' holds two 'aa'; shape follows the cell.
    assert_eq!(
        values(&mut e, "count({'aaaa', 'abab'; 'xyz', ''}, 'aa')"),
        [2., 0., 0., 0.]
    );
    assert_eq!(
        values(&mut e, "size(count({'aaaa', 'abab'; 'xyz', ''}, 'aa'))"),
        [2., 2.]
    );
    assert_eq!(values(&mut e, "count('abcabc', {'bc', 'a'})"), [4.]);
    assert_eq!(values(&mut e, "count(\"Hello hello\", \"hello\")"), [1.]);
    assert_eq!(
        values(
            &mut e,
            "count(\"Hello hello\", \"hello\", 'IgnoreCase', true)"
        ),
        [2.]
    );
    assert_eq!(values(&mut e, "count([\"ab\" missing], \"a\")"), [1., 0.]);
    assert_eq!(values(&mut e, "count('abc', '')"), [0.]);
}

#[test]
fn make_valid_name_follows_the_documented_rules() {
    let mut e = Engine::default();
    e.execute(
        "a = matlab.lang.makeValidName('12 bad name'); \
         b = matlab.lang.makeValidName({'for', 'ok_1', 'a b-c'}); \
         c = matlab.lang.makeValidName(\"a-b\", 'ReplacementStyle', 'delete'); \
         d = matlab.lang.makeValidName('a-b', 'ReplacementStyle', 'hex', 'Prefix', 'p'); \
         f = matlab.lang.makeValidName('1x', 'Prefix', 'p');",
    )
    .unwrap();
    let ok = |e: &mut Engine, check: &str| assert_eq!(values(e, check), [1.], "{check}");
    ok(&mut e, "isequal(a, 'x12BadName')");
    ok(&mut e, "isequal(b, {'xFor', 'ok_1', 'aB_c'})");
    ok(&mut e, "isstring(c) && c == \"ab\"");
    ok(&mut e, "isequal(d, 'a0x2Db')");
    ok(&mut e, "isequal(f, 'p1x')");
    ok(
        &mut e,
        "isequal(matlab.lang.makeValidName('valid_Name1'), 'valid_Name1')",
    );
    ok(
        &mut e,
        "numel(matlab.lang.makeValidName(repmat('a', 1, 80))) == 63",
    );
    assert!(error("matlab.lang.makeValidName(1)").contains("character vector"));
    assert!(error("matlab.lang.makeValidName('a', 'Prefix', '1')").contains("Prefix"));
    assert!(error("matlab.lang.makeValidName('a', 'ReplacementStyle', 'x')").contains("hex"));
}

#[test]
fn lsqminnorm_returns_the_minimum_norm_solution() {
    let mut e = Engine::default();
    close(
        &values(&mut e, "lsqminnorm([1 2; 2 4; 3 6], [1; 2; 3])"),
        &[0.2, 0.4],
    );
    close(
        &values(&mut e, "lsqminnorm([1 0; 0 1; 1 1], [1; 2; 3], 'nowarn')"),
        &[1., 2.],
    );
    // Underdetermined: x = A' * inv(A * A') * b.
    close(&values(&mut e, "lsqminnorm([1 1 1], 3)"), &[1., 1., 1.]);
    // A tolerance above the smaller singular value drops it.
    close(
        &values(&mut e, "lsqminnorm(diag([1 1e-3]), [1; 1], 1e-2)"),
        &[1., 0.],
    );
    assert_eq!(
        values(&mut e, "size(lsqminnorm(zeros(3, 2), ones(3, 4)))"),
        [2., 4.]
    );
    assert!(error("lsqminnorm(ones(3, 2), ones(2, 1))").contains("same number of rows"));
    assert!(error("lsqminnorm(1, 1, -1)").contains("nonnegative"));
}

#[test]
fn givens_matches_octave() {
    let mut e = Engine::default();
    close(&values(&mut e, "givens(3, 4)"), &[0.6, -0.8, 0.8, 0.6]);
    close(&values(&mut e, "givens(-3, 4)"), &[0.6, 0.8, -0.8, 0.6]);
    close(&values(&mut e, "givens(0, 2)"), &[0., -1., 1., 0.]);
    close(&values(&mut e, "givens(0, 0)"), &[1., 0., 0., 1.]);
    e.execute("[c, s] = givens(-1, -1); G = givens(5, -12); r = G * [5; -12];")
        .unwrap();
    close(&values(&mut e, "[c s]"), &[0.5f64.sqrt(), 0.5f64.sqrt()]);
    close(&values(&mut e, "r"), &[13., 0.]);
    assert!(error("givens([1 2], 1)").contains("scalars"));
}

#[test]
fn xcov_is_xcorr_of_mean_removed_sequences() {
    let mut e = Engine::default();
    close(
        &values(&mut e, "xcov([1 2 3 5])"),
        &[-3.9375, -2.125, 1.6875, 8.75, 1.6875, -2.125, -3.9375],
    );
    e.execute("[c, l] = xcov([1 2 3 5], [2 0 1 1], 1, 'coeff');")
        .unwrap();
    close(
        &values(&mut e, "c"),
        &[
            0.4183300132670378,
            -0.23904572186687872,
            -0.23904572186687872,
        ],
    );
    assert_eq!(values(&mut e, "l"), [-1., 0., 1.]);
    // A scalar second argument is maxlag; biased divides by N.
    close(
        &values(&mut e, "xcov([1 2 3 5], 1)"),
        &[1.6875, 8.75, 1.6875],
    );
    close(&values(&mut e, "xcov([1 2 3 5], 0, 'biased')"), &[2.1875]);
    assert!(error("xcov([1 2; 3 4])").contains("xcov: matrix input"));
}

#[test]
fn zplane_draws_zeros_poles_and_the_unit_circle() {
    let mut e = Engine::default();
    e.execute("zplane([1 1], [1 -0.5 0.25]); a = gca;").unwrap();
    // Zeros, poles, two axis lines and the unit circle.
    assert_eq!(values(&mut e, "numel(a.Children)"), [5.]);
    let axes = &e.figure.as_ref().unwrap().axes;
    assert!(axes.extras.equal_aspect);
    let xy = |k: usize, axis: usize| -> Vec<f64> {
        let mut v: Vec<f64> = axes.series[k].points.iter().map(|p| p[axis]).collect();
        v.sort_by(f64::total_cmp);
        v
    };
    // b = [1 1 0] after padding: zeros at 0 and -1; poles 0.25 +- 0.433i.
    close(&xy(1, 0), &[-1., 0.]);
    close(&xy(0, 0), &[0.25, 0.25]);
    close(&xy(0, 1), &[-(0.1875f64.sqrt()), 0.1875f64.sqrt()]);
    e.execute("zplane([0.5; -0.5], [0.9i; -0.9i]);").unwrap();
    let axes = &e.figure.as_ref().unwrap().axes;
    let im: Vec<f64> = axes.series[0].points.iter().map(|p| p[1]).collect();
    close(&im, &[0.9, -0.9]);
    assert!(error("h = zplane([1 1], 1)").contains("not supported yet"));
    assert!(error("zplane([1 Inf], 1)").contains("finite"));
}
