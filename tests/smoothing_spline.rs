//! csaps, fnval, fit(..., 'smoothingspline') and the pp structs of spline, pchip and
//! makima(x, y). Expected values from tool/ref/smoothing_spline.py: SciPy
//! make_smoothing_spline(x, y, w, lam) minimises sum(w (y - g)^2) + lam int g''^2, so the
//! csaps objective p sum(w (y - f)^2) + (1 - p) int f''^2 maps to lam = (1 - p)/p with the
//! same weights (checked there against a dense solve of the Reinsch system); p = 1 is
//! CubicSpline(bc_type='natural'), p = 0 numpy.polyfit(x, y, 1, w=sqrt(w)).
use matrust::{Array, Engine};

const XX: &[f64] = &[
    -0.5,
    0.01666666666666672,
    0.5333333333333334,
    1.0500000000000003,
    1.5666666666666669,
    2.0833333333333335,
    2.6000000000000005,
    3.116666666666667,
    3.6333333333333337,
    4.15,
    4.666666666666667,
    5.183333333333334,
    5.7,
];
const P3: &[f64] = &[
    1.1532334640948765,
    1.4933724327552051,
    1.8330251328457945,
    2.1613772655227854,
    2.4758079607826056,
    2.7672110518369255,
    3.0423288410909404,
    3.3022861502200014,
    3.529843644492022,
    3.736967604848465,
    3.9285332064232352,
    4.101411647465517,
    4.273502750452803,
];
const P9: &[f64] = &[
    0.9390163110191053,
    1.3961168045845715,
    1.8471147166691946,
    2.194201496443291,
    2.5139340837297848,
    2.7138890557456152,
    2.9771521217160144,
    3.3730536781031666,
    3.581492907042705,
    3.788647160434181,
    3.999875789176895,
    4.021091196913041,
    4.032721488801393,
];
const P999: &[f64] = &[
    0.6057617991565515,
    1.2629321292648592,
    1.8326833427563662,
    1.5376717044926402,
    2.832686280687847,
    2.3398359164281635,
    2.1994678455086376,
    3.765597680960466,
    3.3629243710418035,
    3.4336578507941065,
    4.346528048909665,
    3.9354806069187025,
    3.449484960589004,
];
const P1_NATURAL: &[f64] = &[
    0.5620843780218115,
    1.2431917198932656,
    1.813790067705666,
    1.3146121157242443,
    2.92812983934743,
    2.36732046395427,
    2.1000000000000014,
    3.8179492385645086,
    3.3435859569401574,
    3.3857222063786963,
    4.377114034548718,
    3.92946376797218,
    3.4000000000000004,
];
const P0_LINE: &[f64] = &[
    1.370616294203981,
    1.630365695997012,
    1.8901150977900434,
    2.1498644995830745,
    2.4096139013761055,
    2.6693633031691366,
    2.929112704962168,
    3.188862106755199,
    3.4486115085482303,
    3.7083609103412614,
    3.9681103121342924,
    4.227859713927323,
    4.487609115720355,
];
const DEFAULT_P_W: &[f64] = &[0.9749030648242302];
const DEFAULT_W: &[f64] = &[
    0.7934521929043845,
    1.3437969541250512,
    1.876748742060857,
    2.167223808427528,
    2.5421799831120886,
    2.5542601385063626,
    2.7842172446755455,
    3.4626258765571234,
    3.5399683423960897,
    3.7336719586847913,
    4.109511536395841,
    3.9868301180529286,
    3.8372890302745617,
];
const DEFAULT_P: &[f64] = &[0.976196551512714];
const DEFAULT: &[f64] = &[
    0.9615422680062842,
    1.3292455207933431,
    1.6805682191153413,
    1.87308197011813,
    2.3584956322041886,
    2.449262461683757,
    2.619204495211129,
    3.1616981948100533,
    3.3069245672319507,
    3.620182158465737,
    4.080411344710848,
    4.049334037567972,
    3.9921441344545165,
];
const VEC_ROW1: &[f64] = &[
    1.0206187773951783,
    1.3456230271473166,
    1.6663417439951325,
    1.9394068610263844,
    2.2671146045434614,
    2.481109858326349,
    2.7185069003216276,
    3.0676937023995916,
    3.3298231829537857,
    3.6356612023594197,
    3.956748558851897,
    4.110506827393071,
    4.255723999304344,
];
const VEC_ROW2: &[f64] = &[
    0.3195469905465377,
    0.2713671670769806,
    0.23035552636960427,
    0.26566708535037464,
    0.1311748791931132,
    -0.040719137296646016,
    -0.01631204529477713,
    0.31021295202970267,
    0.6266738798935447,
    0.6448040224031233,
    0.3334433854013574,
    -0.1060738367896169,
    -0.5487670051347522,
];
const REPEATED: &[f64] = &[
    1.0923632161968702,
    1.3266201345987898,
    1.5593916715052747,
    1.7960237701762325,
    2.117955182609956,
    2.408936854988109,
    2.7099123026993532,
    3.0502705628509497,
    3.3378491112841875,
    3.6337633861047713,
    3.9259798901245997,
    4.132951338731392,
    4.335683455043405,
];

/// Data of the reference script (rows), its weights and evaluation grid.
const SETUP: &str = "x = [0 0.4 1.1 1.5 2.6 3.0 3.9 4.7 5.2]; \
    y = [1.2 1.9 1.4 2.8 2.1 3.6 3.1 4.4 3.9]; \
    w = [1 2 0.5 1.5 1 3 0.8 1.2 2]; \
    y2 = [0.5 -0.3 0.8 0.1 -0.6 0.4 1.1 0.2 -0.2];";

fn run(e: &mut Engine, source: &str) -> String {
    e.execute(source)
        .unwrap_or_else(|err| panic!("{source}: {err}"))
        .output
        .join("")
}
fn error(e: &mut Engine, source: &str) -> String {
    e.execute(source).expect_err(source)
}
fn numbers(e: &Engine, name: &str) -> Vec<f64> {
    match e.workspace().get(name).unwrap() {
        Array::Numeric(x) => x.data().to_vec(),
        Array::Logical(x) => x.data().iter().map(|v| f64::from(*v)).collect(),
        x => panic!("{name}: unexpected {}", x.class()),
    }
}
fn size(e: &Engine, name: &str) -> (usize, usize) {
    let ws = e.workspace();
    let a = ws.get(name).unwrap();
    (a.rows(), a.cols())
}
fn text(e: &Engine, name: &str) -> String {
    match e.workspace().get(name).unwrap() {
        Array::Character(c) => String::from_utf16_lossy(c.units()),
        x => panic!("{name}: unexpected {}", x.class()),
    }
}
/// Relative closeness |a - b| <= tol * (1 + |b|).
fn close(what: &str, got: &[f64], want: &[f64], tol: f64) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?} vs {want:?}");
    for (i, (a, b)) in got.iter().zip(want).enumerate() {
        assert!(
            (a - b).abs() <= tol * (1. + b.abs()),
            "{what}[{i}]: got {a}, expected {b}"
        );
    }
}
fn row(v: &[f64]) -> String {
    let parts: Vec<String> = v.iter().map(|x| format!("{x:?}")).collect();
    format!("[{}]", parts.join(" "))
}
fn engine() -> Engine {
    let mut e = Engine::default();
    run(&mut e, SETUP);
    run(&mut e, &format!("xx = {};", row(XX)));
    e
}

#[test]
fn explicit_p_matches_scipy_make_smoothing_spline() {
    let mut e = engine();
    for (p, want) in [("0.3", P3), ("0.9", P9), ("0.999", P999)] {
        run(
            &mut e,
            &format!(
                "v = csaps(x, y, {p}, xx, w); pp = csaps(x, y, {p}, [], w); a = ppval(pp, xx); b = fnval(pp, xx);"
            ),
        );
        close(&format!("p = {p}"), &numbers(&e, "v"), want, 1e-9);
        assert_eq!(size(&e, "v"), (1, 13));
        close("ppval", &numbers(&e, "a"), &numbers(&e, "v"), 1e-14);
        close("fnval", &numbers(&e, "b"), &numbers(&e, "v"), 1e-14);
    }
}

#[test]
fn p_one_is_the_natural_interpolant_and_p_zero_the_weighted_line() {
    let mut e = engine();
    run(
        &mut e,
        "v1 = csaps(x, y, 1, xx); s = csaps(x, y, 1, x, w); v0 = csaps(x, y, 0, xx, w);",
    );
    close("p = 1", &numbers(&e, "v1"), P1_NATURAL, 1e-9);
    close(
        "p = 1 at the sites",
        &numbers(&e, "s"),
        &numbers(&e, "y"),
        1e-12,
    );
    close("p = 0", &numbers(&e, "v0"), P0_LINE, 1e-9);
    // The p = 0 pieces are the line: no quadratic or cubic terms.
    run(&mut e, "pp = csaps(x, y, 0, [], w); c = pp.coefs(:, 1:2);");
    assert!(numbers(&e, "c").iter().all(|v| *v == 0.));
}

#[test]
fn default_p_follows_the_trace_rule() {
    let mut e = engine();
    run(
        &mut e,
        "[pp, P] = csaps(x, y, [], [], w); a = ppval(pp, xx); \
         [pp1, P1] = csaps(x, y); b = fnval(pp1, xx); \
         [v, P2] = csaps(x, y, [], xx); [~, P3] = csaps(x, y, 0.3);",
    );
    close("P with weights", &numbers(&e, "P"), DEFAULT_P_W, 1e-12);
    close("values with weights", &numbers(&e, "a"), DEFAULT_W, 1e-9);
    close("P", &numbers(&e, "P1"), DEFAULT_P, 1e-12);
    close("values", &numbers(&e, "b"), DEFAULT, 1e-9);
    close("[v, P]", &numbers(&e, "v"), DEFAULT, 1e-9);
    close("P from the xx form", &numbers(&e, "P2"), DEFAULT_P, 1e-12);
    assert_eq!(numbers(&e, "P3"), vec![0.3]);
    // Uniform spacing h: trace(R) = 4h(n-2), trace(Q'Q) = 6(n-2)/h^2, so p = 1/(1 + h^3/9).
    run(&mut e, "[~, Pu] = csaps(0:0.5:5, sin(0:0.5:5));");
    close(
        "uniform",
        &numbers(&e, "Pu"),
        &[1. / (1. + 0.125 / 9.)],
        1e-13,
    );
    // Two sites: the line through them, P = 1.
    run(
        &mut e,
        "[pp2, P4] = csaps([1 3], [2 6]); l = ppval(pp2, [0 2 4]);",
    );
    assert_eq!(numbers(&e, "P4"), vec![1.]);
    close("two sites", &numbers(&e, "l"), &[0., 4., 8.], 1e-15);
}

#[test]
fn pp_is_the_mkpp_struct() {
    let mut e = engine();
    run(
        &mut e,
        "pp = csaps(x, y, 0.9); f = pp.form; b = pp.breaks; n = pp.pieces; k = pp.order; d = pp.dim; \
         c = pp.coefs; [b2, c2, l2, k2, d2] = unmkpp(pp); q = ppval(mkpp(b2, c2), xx); r = ppval(pp, xx);",
    );
    assert_eq!(text(&e, "f"), "pp");
    assert_eq!(numbers(&e, "b"), numbers(&e, "x"));
    assert_eq!(size(&e, "b"), (1, 9));
    assert_eq!(numbers(&e, "n"), vec![8.]);
    assert_eq!(numbers(&e, "k"), vec![4.]);
    assert_eq!(numbers(&e, "d"), vec![1.]);
    assert_eq!(size(&e, "c"), (8, 4));
    assert_eq!(numbers(&e, "q"), numbers(&e, "r"));
}

#[test]
fn vector_valued_y_smooths_each_row() {
    let mut e = engine();
    run(
        &mut e,
        "v = csaps(x, [y; y2], 0.9, xx); pp = csaps(x, [y; y2], 0.9); d = pp.dim; a = ppval(pp, xx); \
         c = pp.coefs; r1 = v(1, :); r2 = v(2, :);",
    );
    assert_eq!(size(&e, "v"), (2, 13));
    close("row 1", &numbers(&e, "r1"), VEC_ROW1, 1e-9);
    close("row 2", &numbers(&e, "r2"), VEC_ROW2, 1e-9);
    assert_eq!(numbers(&e, "d"), vec![2.]);
    assert_eq!(size(&e, "c"), (16, 4));
    close("ppval", &numbers(&e, "a"), &numbers(&e, "v"), 1e-14);
}

#[test]
fn unsorted_repeated_and_nan_sites() {
    let mut e = engine();
    // 1.1 appears twice (1.2 and 1.6): averaged to 1.4 with weight 2.
    run(
        &mut e,
        "xu = [2.6 1.1 0 1.5 0.4 1.1 3.0 5.2 3.9 4.7]; yu = [2.1 1.2 1.2 2.8 1.9 1.6 3.6 3.9 3.1 4.4]; \
         v = csaps(xu, yu, 0.8, xx); pp = csaps(xu', yu', 0.8); b = pp.breaks;",
    );
    close("repeated", &numbers(&e, "v"), REPEATED, 1e-9);
    assert_eq!(numbers(&e, "b"), numbers(&e, "x"));
    run(
        &mut e,
        "a = csaps([x NaN 2], [y 5 NaN], 0.9, xx, [w 1 1]); b = csaps(x, y, 0.9, xx, w); \
         c = csaps([x 7], [y NaN], [], xx); d = csaps(x, y, [], xx);",
    );
    close(
        "NaN pairs removed",
        &numbers(&e, "a"),
        &numbers(&e, "b"),
        1e-15,
    );
    close(
        "NaN pairs removed, default p",
        &numbers(&e, "c"),
        &numbers(&e, "d"),
        1e-15,
    );
}

#[test]
fn fnval_takes_either_order_and_matrix_points() {
    let mut e = engine();
    run(
        &mut e,
        "pp = csaps(x, y, 0.9); a = fnval(pp, xx); b = fnval(xx, pp); m = fnval(pp, [0.5 1; 2 3]); \
         m2 = ppval(pp, [0.5 1; 2 3]);",
    );
    assert_eq!(numbers(&e, "a"), numbers(&e, "b"));
    assert_eq!(size(&e, "m"), (2, 2));
    assert_eq!(numbers(&e, "m"), numbers(&e, "m2"));
}

#[test]
fn fit_smoothingspline_matches_csaps() {
    let mut e = engine();
    run(
        &mut e,
        "[f, g, o] = fit(x', y', 'smoothingspline', 'Weights', w'); a = f(xx'); p = o.p; \
         t = type(f); k = category(f); dfe = g.dfe; sse = g.sse; adj = g.adjrsquare; rm = g.rmse; \
         sse2 = sum(w' .* (y' - f(x')).^2); ci = confint(f);",
    );
    close("default p", &numbers(&e, "p"), DEFAULT_P_W, 1e-12);
    close("default values", &numbers(&e, "a"), DEFAULT_W, 1e-9);
    assert_eq!(text(&e, "t"), "smoothingspline");
    assert_eq!(text(&e, "k"), "spline");
    assert_eq!(numbers(&e, "dfe"), vec![0.]);
    close("sse", &numbers(&e, "sse"), &numbers(&e, "sse2"), 1e-12);
    assert!(numbers(&e, "sse")[0] > 0.);
    assert!(numbers(&e, "adj")[0].is_nan() && numbers(&e, "rm")[0].is_nan());
    assert!(numbers(&e, "ci").iter().all(|v| v.is_nan()));
    run(
        &mut e,
        "[f9, ~, o9] = fit(x', y', 'smoothingspline', 'SmoothingParam', 0.9, 'Weights', w'); b = f9(xx'); p9 = o9.p; \
         opts = fitoptions('smoothingspline', 'SmoothingParam', 0.3); f3 = fit(x', y', 'smoothingspline', opts); \
         c = f3(xx'); c2 = csaps(x, y, 0.3, xx)'; \
         ft = fittype('smoothingspline'); f0 = fit(x', y', ft); d = f0(xx'); d2 = csaps(x, y, [], xx)';",
    );
    close("SmoothingParam 0.9", &numbers(&e, "b"), P9, 1e-9);
    assert_eq!(numbers(&e, "p9"), vec![0.9]);
    close("fitoptions", &numbers(&e, "c"), &numbers(&e, "c2"), 1e-13);
    close("fittype", &numbers(&e, "d"), &numbers(&e, "d2"), 1e-13);
    let shown = run(&mut e, "f9");
    assert!(shown.contains("Smoothing spline"), "{shown}");
    assert!(
        shown.contains("Smoothing parameter") && shown.contains("p = 0.9"),
        "{shown}"
    );
}

#[test]
fn fit_smoothingspline_options() {
    let mut e = engine();
    run(
        &mut e,
        "o = fitoptions('smoothingspline'); m = o.Method; s = o.SmoothingParam; \
         o2 = fitoptions('Method', 'SmoothingSpline', 'SmoothingParam', 0.5); s2 = o2.SmoothingParam; \
         f = fit(x', y', 'smoothingspline', 'Exclude', 3, 'SmoothingParam', 0.9); a = f(xx'); \
         xe = x([1:2 4:end]); ye = y([1:2 4:end]); b = csaps(xe, ye, 0.9, xx)'; \
         fn = fit(x', y', 'smoothingspline', 'Normalize', 'on', 'SmoothingParam', 0.9); \
         c = fn(xx'); c2 = csaps((x - mean(x)) / std(x), y, 0.9, (xx - mean(x)) / std(x))';",
    );
    assert_eq!(text(&e, "m"), "SmoothingSpline");
    assert!(numbers(&e, "s").is_empty());
    assert_eq!(numbers(&e, "s2"), vec![0.5]);
    close("Exclude", &numbers(&e, "a"), &numbers(&e, "b"), 1e-13);
    close("Normalize", &numbers(&e, "c"), &numbers(&e, "c2"), 1e-12);
    for (code, expected) in [
        (
            "fit(x', y', 'smoothingspline', 'SmoothingParam', 2)",
            "between 0 and 1",
        ),
        (
            "fit(x', y', 'smoothingspline', 'Robust', 'on')",
            "not an option of the SmoothingSpline",
        ),
        (
            "fitoptions('poly1', 'SmoothingParam', 0.5)",
            "not an option of the LinearLeastSquares",
        ),
    ] {
        let message = error(&mut e, code);
        assert!(message.contains(expected), "{code}: {message}");
    }
}

#[test]
fn spline_pchip_makima_two_inputs_return_pp() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x = [0 1 2.5 3 4.5 6]; y = [1 3 2 5 4 0]; q = linspace(-1, 7, 33);",
    );
    for name in ["spline", "pchip", "makima"] {
        run(
            &mut e,
            &format!(
                "pp = {name}(x, y); a = ppval(pp, q); b = {name}(x, y, q); f = pp.form; k = pp.order;"
            ),
        );
        close(name, &numbers(&e, "a"), &numbers(&e, "b"), 1e-12);
        assert_eq!(text(&e, "f"), "pp");
        assert_eq!(numbers(&e, "k"), vec![4.]);
    }
    // Unsorted sites, the three- and two-point spline, and makima with rows as components.
    run(
        &mut e,
        "a = ppval(spline([3 0 1 2], [27 0 1 8]), 1.5); b = ppval(spline([0 1 3], [1 0 4]), q); \
         b2 = spline([0 1 3], [1 0 4], q); c = ppval(pchip([0 2], [1 5]), q); c2 = pchip([0 2], [1 5], q); \
         pm = makima(x, [y; y.^2]); m = ppval(pm, q); m2 = makima(x, [y; y.^2], q); dm = pm.dim;",
    );
    close("cubic data", &numbers(&e, "a"), &[3.375], 1e-14);
    close("three points", &numbers(&e, "b"), &numbers(&e, "b2"), 1e-12);
    close("two points", &numbers(&e, "c"), &numbers(&e, "c2"), 1e-12);
    assert_eq!(numbers(&e, "dm"), vec![2.]);
    assert_eq!(size(&e, "m"), (2, 33));
    close("makima rows", &numbers(&e, "m"), &numbers(&e, "m2"), 1e-12);
}

#[test]
fn errors_are_reported() {
    let mut e = engine();
    for (code, expected) in [
        ("csaps(x, y, 1.5)", "between 0 and 1"),
        ("csaps(x, y, -0.1)", "between 0 and 1"),
        ("csaps(x, y, [0.1 0.2])", "between 0 and 1"),
        ("csaps(x, y(1:5))", "same number of data sites"),
        ("csaps(1, 2)", "at least two distinct data sites"),
        (
            "csaps([1 1 1], [2 3 4])",
            "at least two distinct data sites",
        ),
        ("csaps([1 NaN], [2 3])", "at least two distinct data sites"),
        ("csaps(x, y, 0.5, [], w(1:3))", "one value per data site"),
        ("csaps(x, y, 0.5, [], -w)", "nonnegative"),
        ("csaps(x, [y; y2]', 0.5)", "one column per data site"),
        ("csaps([x Inf], [y 1])", "finite"),
        ("csaps({x, x}, y)", "not supported"),
        (
            "csaps([0 1e-14 1 1+1e-14 2 3 4], [0 1 0 1 0 1 0], 0.5)",
            "numerically singular",
        ),
        ("csaps(x)", "Not enough input arguments"),
        ("fnval(1, 2)", "piecewise polynomial"),
        ("fnval(csaps(x, y), xx, 'l')", "not supported"),
        ("spline(1, 2)", "at least two data points"),
    ] {
        let message = error(&mut e, code);
        assert!(message.contains(expected), "{code}: {message}");
    }
}

#[test]
fn registered_as_builtins() {
    for name in ["csaps", "fnval"] {
        assert!(matrust::session::FUNCTIONS.contains(&name), "{name}");
    }
}
