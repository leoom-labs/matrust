//! Image registration piece 7c2: imwarp.
//!
//! Reference values: tests/fixtures/registration/imwarp.json, written by
//! tool/refs/registration/imwarp_ref.py (NumPy on the documented grid, each
//! case cross-checked against scipy.ndimage.map_coordinates and scikit-image
//! 0.26 `warp` on the same points; no MATLAB). Tolerance 1e-6 absolute for
//! linear and cubic (relative for single), exact for nearest, logical and
//! uint8. Nearest samples that land on a rounding tie (the centre pixel of an
//! odd-sized view maps onto the centre of the even-sized 16x20 input) accept
//! every value the tie can round to.
//!
//! The other expected values are derived by hand from the public MathWorks
//! imwarp and imref2d reference pages, written next to each check.
use matrust::{Array, Engine, Matrix};
use serde_json::Value as Json;

fn matrix(e: &Engine, name: &str) -> Matrix {
    if let Some(m) = e.variables.get(name) {
        return m.clone();
    }
    match e.array(name) {
        Some(Array::Numeric(m)) => m.clone(),
        other => panic!("missing numeric {name}: {other:?}"),
    }
}

fn run(e: &mut Engine, code: &str) {
    if let Err(error) = e.execute(code) {
        panic!("{code}\n  failed: {error}");
    }
}

fn error(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(_) => panic!("{code}\n  succeeded; expected an error"),
        Err(error) => error,
    }
}

fn error_has(e: &mut Engine, code: &str, part: &str) {
    let message = error(e, code);
    assert!(
        message.contains(part),
        "{code}\n  error {message:?} does not contain {part:?}"
    );
}

/// Size and column-major values of an expression (as double).
fn values(e: &mut Engine, expr: &str) -> (Vec<usize>, Vec<f64>) {
    run(
        e,
        &format!("zz_v = double({expr}); zz_d = size(zz_v); zz_v = zz_v(:)';"),
    );
    let dims = matrix(e, "zz_d")
        .data()
        .iter()
        .map(|&d| d as usize)
        .collect();
    (dims, matrix(e, "zz_v").data().to_vec())
}

fn class_of(e: &mut Engine, expr: &str) -> String {
    run(e, &format!("zz_c = double(class({expr}));"));
    matrix(e, "zz_c")
        .data()
        .iter()
        .map(|&c| char::from_u32(c as u32).unwrap())
        .collect()
}

fn assert_values(e: &mut Engine, expr: &str, dims: &[usize], expected: &[f64], tol: f64) {
    let (d, v) = values(e, expr);
    assert_eq!(d, dims, "{expr}: size");
    for (i, (a, b)) in v.iter().zip(expected).enumerate() {
        let ok = a == b || a.is_nan() && b.is_nan() || (a - b).abs() <= tol;
        assert!(ok, "{expr}: element {i} is {a}, expected {b}");
    }
}

fn numbers(v: &Json) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn fixture() -> Json {
    serde_json::from_str(include_str!("fixtures/registration/imwarp.json")).unwrap()
}

/// Defines each fixture input in chunks of 100 values (one statement may
/// hold at most 256 expression nodes).
fn engine_with_inputs(json: &Json) -> Engine {
    let mut e = Engine::default();
    for (var, input) in json["inputs"].as_object().unwrap() {
        let class = input["class"].as_str().unwrap();
        let dims: Vec<String> = numbers(&input["dims"])
            .iter()
            .map(|d| d.to_string())
            .collect();
        run(&mut e, &format!("{var} = [];"));
        for chunk in numbers(&input["data"]).chunks(100) {
            let items: Vec<String> = chunk.iter().map(|v| format!("{v:?}")).collect();
            run(&mut e, &format!("{var} = [{var} {}];", items.join(" ")));
        }
        let shaped = format!("reshape({var}, [{}])", dims.join(" "));
        let value = if class == "double" {
            shaped
        } else {
            format!("{class}({shaped})")
        };
        run(&mut e, &format!("{var} = {value};"));
    }
    e
}

#[test]
fn imwarp_matches_the_reference_cases() {
    let json = fixture();
    let mut e = engine_with_inputs(&json);
    let cases = json["cases"].as_array().unwrap();
    assert!(cases.len() >= 29, "only {} cases", cases.len());
    let mut worst = 0f64;
    let mut ties = 0;
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let expr = case["expr"].as_str().unwrap();
        run(&mut e, case["setup"].as_str().unwrap());
        run(&mut e, &format!("x = {expr};"));
        let expected = &case["expected"];
        let class = expected["class"].as_str().unwrap();
        assert_eq!(class_of(&mut e, "x"), class, "{name}: {expr} class");
        let dims: Vec<usize> = numbers(&expected["dims"])
            .iter()
            .map(|&d| d as usize)
            .collect();
        let want = numbers(&expected["data"]);
        let tol = case["tol"].as_f64().unwrap();
        let alternatives = case["ties"].as_object().unwrap();
        let (d, got) = values(&mut e, "x");
        assert_eq!(d, dims, "{name}: {expr} size");
        assert_eq!(got.len(), want.len(), "{name}: element count");
        for (i, (a, b)) in got.iter().zip(&want).enumerate() {
            if let Some(options) = alternatives.get(&i.to_string()) {
                ties += 1;
                assert!(
                    numbers(options).contains(a),
                    "{name}: {expr} tie element {i} is {a}, expected one of {options}"
                );
                continue;
            }
            let err = (a - b).abs();
            let limit = if class == "single" {
                tol * b.abs().max(1.)
            } else {
                tol
            };
            assert!(
                err <= limit || a.is_nan() && b.is_nan(),
                "{name}: {expr} element {i} is {a}, expected {b}"
            );
            if !err.is_nan() {
                worst = worst.max(err);
            }
        }
        // RB: the default (or given) output view.
        let rb = &case["rb"];
        if rb.is_null() {
            continue;
        }
        run(&mut e, &format!("[x2, RB] = {expr};"));
        run(
            &mut e,
            "zz_same = double(isequaln(x(:), x2(:)) && isequal(size(x), size(x2)));",
        );
        assert_eq!(matrix(&e, "zz_same").data(), [1.], "{name}: B of [B, RB]");
        assert_eq!(class_of(&mut e, "RB"), "imref2d", "{name}: RB class");
        for (property, key) in [
            ("ImageSize", "size"),
            ("XWorldLimits", "x"),
            ("YWorldLimits", "y"),
        ] {
            let (_, got) = values(&mut e, &format!("RB.{property}"));
            let want = numbers(&rb[key]);
            for (a, b) in got.iter().zip(&want) {
                assert!(
                    (a - b).abs() <= 1e-12 * b.abs().max(1.),
                    "{name}: RB.{property} is {got:?}, expected {want:?}"
                );
            }
        }
    }
    eprintln!("imwarp reference cases: largest abs error {worst:e}, {ties} tie pixels");
}

#[test]
fn default_view_keeps_the_input_pixel_extent_and_centres_the_box() {
    let mut e = Engine::default();
    // Translation by (2.5, -1.25): limits [0.5 20.5]+2.5 and [0.5 16.5]-1.25,
    // whole pixels, so no widening; B is A itself.
    run(
        &mut e,
        "A = magic(20); A = A(1:16, :); [B, RB] = imwarp(A, transltform2d(2.5, -1.25));",
    );
    assert_values(&mut e, "RB.XWorldLimits", &[1, 2], &[3., 23.], 0.);
    assert_values(&mut e, "RB.YWorldLimits", &[1, 2], &[-0.75, 15.25], 0.);
    assert_values(&mut e, "RB.ImageSize", &[1, 2], &[16., 20.], 0.);
    run(&mut e, "zz = double(isequal(A, B));");
    assert_eq!(matrix(&e, "zz").data(), [1.]);
    // Rotation by 90 degrees of a 3-by-4 image: corners (0.5..4.5, 0.5..3.5)
    // map to x = -y, y = x: XWorldLimits [-3.5 -0.5], YWorldLimits [0.5 4.5];
    // the output is 4-by-3 and equals rot90(A, -1) (y points down, so a
    // counterclockwise world rotation turns the picture clockwise).
    run(
        &mut e,
        "A = reshape(1:12, 3, 4); [B, RB] = imwarp(A, rigidtform2d(90, [0 0]));",
    );
    assert_values(&mut e, "RB.XWorldLimits", &[1, 2], &[-3.5, -0.5], 1e-15);
    assert_values(&mut e, "RB.YWorldLimits", &[1, 2], &[0.5, 4.5], 1e-15);
    run(&mut e, "zz = double(isequal(B, rot90(A, -1)));");
    assert_eq!(matrix(&e, "zz").data(), [1.]);
    // Scale 1.5 (affinetform2d) of a 4-by-4 image with pixel extent 1: limits
    // 1.5*[0.5 4.5] = [0.75 6.75], span 6, 6 pixels, no widening.
    run(
        &mut e,
        "[B, RB] = imwarp(ones(4), affinetform2d([1.5 0 0; 0 1.5 0; 0 0 1]));",
    );
    assert_values(&mut e, "RB.XWorldLimits", &[1, 2], &[0.75, 6.75], 1e-15);
    assert_values(&mut e, "size(B)", &[1, 2], &[6., 6.], 0.);
    // Scale 1.3 of a 2-by-2 image: [0.65 3.25], span 2.6 -> ceil = 3 pixels,
    // widened by (3 - 2.6)/2 = 0.2 on each side: [0.45 3.45].
    run(
        &mut e,
        "[B, RB] = imwarp(ones(2), affinetform2d([1.3 0 0; 0 1.3 0; 0 0 1]));",
    );
    assert_values(&mut e, "RB.XWorldLimits", &[1, 2], &[0.45, 3.45], 1e-14);
    assert_values(&mut e, "RB.YWorldLimits", &[1, 2], &[0.45, 3.45], 1e-14);
    assert_values(&mut e, "RB.ImageSize", &[1, 2], &[3., 3.], 0.);
    // RA with pixel extent 2: the output keeps extent 2. imref2d([2 3], 2, 2)
    // has limits [1 7] x [1 5]; a translation by (1, 0) gives [2 8] x [1 5].
    run(
        &mut e,
        "M = magic(3); M = M(1:2, :); [B, RB] = imwarp(M, imref2d([2 3], 2, 2), transltform2d(1, 0));",
    );
    assert_values(&mut e, "RB.XWorldLimits", &[1, 2], &[2., 8.], 0.);
    assert_values(&mut e, "RB.PixelExtentInWorldX", &[1, 1], &[2.], 0.);
    assert_values(&mut e, "B", &[2, 3], &[8., 3., 1., 5., 6., 7.], 0.);
}

#[test]
fn output_view_sets_size_and_location() {
    let mut e = Engine::default();
    // Identity with a view one pixel larger on the right and bottom: the extra
    // pixel centres (x = 4 or y = 4) lie outside [1, 3] -> FillValues 9.
    run(
        &mut e,
        "A = magic(3); B = imwarp(A, rigidtform2d(), 'OutputView', imref2d([4 4]), 'FillValues', 9);",
    );
    assert_values(
        &mut e,
        "B",
        &[4, 4],
        &[
            8., 3., 4., 9., 1., 5., 9., 9., 6., 7., 2., 9., 9., 9., 9., 9.,
        ],
        0.,
    );
    // A view shifted by half a pixel samples between the pixels (linear):
    // centres x = 1.5, 2.5 between columns -> means of neighbours.
    run(
        &mut e,
        "B = imwarp([0 10 20], transltform2d(0, 0), 'OutputView', imref2d([1 2], [1 3], [0.5 1.5]));",
    );
    assert_values(&mut e, "B", &[1, 2], &[5., 15.], 1e-15);
    // Positional method and name-value InterpolationMethod agree.
    run(
        &mut e,
        "t = rigidtform2d(17, [0.3 -0.2]); A = magic(6); B1 = imwarp(A, t, 'cubic'); B2 = imwarp(A, t, 'InterpolationMethod', 'cubic'); zz = double(isequal(B1, B2));",
    );
    assert_eq!(matrix(&e, "zz").data(), [1.]);
    // Parameter names: any case, unique prefixes.
    run(
        &mut e,
        "B3 = imwarp(A, t, 'interpolationmethod', 'CUBIC', 'outputview', imref2d(size(B1), [0.5 size(B1, 2)+0.5], [0.5 size(B1, 1)+0.5]), 'Fill', 0, 'Smooth', false);",
    );
}

#[test]
fn edges_fill_values_and_smooth_edges() {
    let mut e = Engine::default();
    // Shift by half a pixel into a same-size view: output column 1 samples
    // x = 0.5, outside the pixel-centre hull [1, 4] -> fill (0); the rest
    // read x = 1.5, 2.5, 3.5 (means of neighbours).
    run(
        &mut e,
        "A = repmat([2 4 6 8], 2, 1); t = transltform2d(0.5, 0); V = imref2d([2 4]);",
    );
    assert_values(
        &mut e,
        "imwarp(A, t, 'OutputView', V)",
        &[2, 4],
        &[0., 0., 3., 3., 5., 5., 7., 7.],
        1e-15,
    );
    // SmoothEdges pads with FillValues: x = 0.5 blends the pad (fill 10) and
    // A(:, 1) = 2 half and half -> 6.
    assert_values(
        &mut e,
        "imwarp(A, t, 'OutputView', V, 'FillValues', 10, 'SmoothEdges', true)",
        &[2, 4],
        &[6., 6., 3., 3., 5., 5., 7., 7.],
        1e-15,
    );
    // Nearest: x = 0.4 is outside the hull [1, 4] -> fill; 1.4, 2.4, 3.4
    // round to columns 1, 2, 3. With SmoothEdges the hull is [-1, 6]: x = 0.6,
    // 1.6, 2.6, 3.6 round to columns 1..4 (without it 0.6 would be fill).
    assert_values(
        &mut e,
        "imwarp(A, transltform2d(0.6, 0), 'nearest', 'OutputView', V, 'FillValues', -1)",
        &[2, 4],
        &[-1., -1., 2., 2., 4., 4., 6., 6.],
        0.,
    );
    assert_values(
        &mut e,
        "imwarp(A, transltform2d(0.4, 0), 'nearest', 'OutputView', V, 'SmoothEdges', 1)",
        &[2, 4],
        &[2., 2., 4., 4., 6., 6., 8., 8.],
        0.,
    );
    // NaN fill.
    assert_values(
        &mut e,
        "imwarp(A, t, 'OutputView', V, 'FillValues', NaN)",
        &[2, 4],
        &[f64::NAN, f64::NAN, 3., 3., 5., 5., 7., 7.],
        1e-15,
    );
    // Per-channel FillValues on an RGB image; a scalar fills every channel.
    run(
        &mut e,
        "rgb = uint8(cat(3, 10*ones(2), 20*ones(2), 30*ones(2))); W = imref2d([2 3]);",
    );
    assert_values(
        &mut e,
        "imwarp(rgb, rigidtform2d(), 'OutputView', W, 'FillValues', [255 0 7])",
        &[2, 3, 3],
        &[
            10., 10., 10., 10., 255., 255., 20., 20., 20., 20., 0., 0., 30., 30., 30., 30., 7., 7.,
        ],
        0.,
    );
    assert_values(
        &mut e,
        "imwarp(rgb, rigidtform2d(), 'OutputView', W, 'FillValues', 5)",
        &[2, 3, 3],
        &[
            10., 10., 10., 10., 5., 5., 20., 20., 20., 20., 5., 5., 30., 30., 30., 30., 5., 5.,
        ],
        0.,
    );
}

#[test]
fn classes_round_saturate_and_threshold() {
    let mut e = Engine::default();
    // Identity keeps every class and value.
    for class in ["uint8", "uint16", "int16", "int32", "single", "logical"] {
        run(
            &mut e,
            &format!("A = {class}([1 0 1; 0 1 1]); B = imwarp(A, transltform2d(0, 0));"),
        );
        assert_eq!(class_of(&mut e, "B"), class, "identity of {class}");
        assert_values(&mut e, "B", &[2, 3], &[1., 0., 0., 1., 1., 1.], 0.);
    }
    // uint8: linear midpoints round half away from zero (1.5 -> 2) and the
    // cubic overshoot at a 0 -> 255 step saturates to [0, 255].
    run(
        &mut e,
        "B = imwarp(uint8([1 2 3 4]), transltform2d(0.5, 0), 'OutputView', imref2d([1 4]));",
    );
    assert_values(&mut e, "B", &[1, 4], &[0., 2., 3., 4.], 0.);
    run(
        &mut e,
        "S = uint8([0 0 0 255 255 255]); C = imwarp(S, transltform2d(0.3, 0), 'cubic', 'OutputView', imref2d([1 6])); Cd = imwarp(double(S), transltform2d(0.3, 0), 'cubic', 'OutputView', imref2d([1 6]));",
    );
    let (_, c) = values(&mut e, "C");
    let (_, cd) = values(&mut e, "Cd");
    assert!(cd.iter().any(|&v| v < 0.) && cd.iter().any(|&v| v > 255.));
    for (a, b) in c.iter().zip(&cd) {
        let want = b.clamp(0., 255.);
        let want = want.signum() * (want.abs() + 0.5).floor();
        assert_eq!(*a, want, "uint8 cubic {c:?} from {cd:?}");
    }
    // int16 fill -3.6 rounds to -4.
    run(
        &mut e,
        "B = imwarp(int16([5 6]), rigidtform2d(), 'OutputView', imref2d([1 3]), 'FillValues', -3.6);",
    );
    assert_values(&mut e, "B", &[1, 3], &[5., 6., -4.], 0.);
    // single stays single (sample in double, stored as single).
    run(
        &mut e,
        "B = imwarp(single([0 1]), transltform2d(0.25, 0), 'OutputView', imref2d([1 2], [0.75 2.75], [0.5 1.5]));",
    );
    assert_eq!(class_of(&mut e, "B"), "single");
    assert_values(&mut e, "B", &[1, 2], &[0., 1.], 0.);
    // logical: interpolated as 0/1 and thresholded at 0.5; result logical.
    run(
        &mut e,
        "L = logical([0 1 1 0]); B = imwarp(L, transltform2d(0.4, 0), 'OutputView', imref2d([1 4]));",
    );
    assert_eq!(class_of(&mut e, "B"), "logical");
    // x = 0.6 -> fill; 1.6 -> 0.6 (true); 2.6 -> 1 (true); 3.6 -> 0.4 (false).
    assert_values(&mut e, "B", &[1, 4], &[0., 1., 1., 0.], 0.);
}

#[test]
fn displacement_field_adds_to_columns_and_rows() {
    let mut e = Engine::default();
    // D(:,:,1) = 1 shifts every sample one column to the right (B(:, j) =
    // A(:, j+1)); the last column falls outside -> 0.
    run(
        &mut e,
        "A = magic(4); D = cat(3, ones(4), zeros(4)); B = imwarp(A, D);",
    );
    assert_values(
        &mut e,
        "B",
        &[4, 4],
        &[
            2., 11., 7., 14., 3., 10., 6., 15., 13., 8., 12., 1., 0., 0., 0., 0.,
        ],
        0.,
    );
    // D(:,:,2) = -0.5 samples half a row up: row 1 at y = 0.5 is outside the
    // hull -> fill 100; the other rows are means of neighbouring rows.
    run(
        &mut e,
        "B = imwarp(A, cat(3, zeros(4), -0.5*ones(4)), 'FillValues', 100);",
    );
    let (_, b) = values(&mut e, "B");
    let a = [
        16., 5., 9., 4., 2., 11., 7., 14., 3., 10., 6., 15., 13., 8., 12., 1.,
    ];
    for col in 0..4 {
        assert_eq!(b[col * 4], 100.);
        for row in 1..4 {
            let want = (a[col * 4 + row - 1] + a[col * 4 + row]) / 2.;
            assert!((b[col * 4 + row] - want).abs() < 1e-14);
        }
    }
    // The output has the size of D; RGB channels are warped alike.
    run(
        &mut e,
        "rgb = uint8(cat(3, magic(4), 2*magic(4), 3*magic(4))); B = imwarp(rgb, zeros(2, 3, 2), 'nearest');",
    );
    assert_values(&mut e, "size(B)", &[1, 3], &[2., 3., 3.], 0.);
    assert_eq!(class_of(&mut e, "B"), "uint8");
    assert_values(
        &mut e,
        "B(:, :, 2)",
        &[2, 3],
        &[32., 10., 4., 22., 6., 20.],
        0.,
    );
    error_has(
        &mut e,
        "[B, RB] = imwarp(A, D);",
        "displacement field gives one output",
    );
    error_has(
        &mut e,
        "imwarp(A, D, 'OutputView', imref2d([4 4]))",
        "OutputView is not used with a displacement field",
    );
}

#[test]
fn calls_through_handles_statements_and_rgb() {
    let mut e = Engine::default();
    run(
        &mut e,
        "rgb = uint8(cat(3, magic(5), magic(5), magic(5))); h = @imwarp; B = h(rgb, rigidtform2d(30, [0 0]));",
    );
    assert_eq!(class_of(&mut e, "B"), "uint8");
    let (dims, _) = values(&mut e, "B");
    assert_eq!(dims.len(), 3);
    assert_eq!(dims[2], 3);
    run(&mut e, "imwarp(magic(3), transltform2d(1, 1));");
    run(
        &mut e,
        "C = cellfun(@(x) size(imwarp(x, transltform2d(1, 0)), 1), {ones(2), ones(3)});",
    );
    assert_values(&mut e, "C", &[1, 2], &[2., 3.], 0.);
}

#[test]
fn bad_calls_are_explained() {
    let mut e = Engine::default();
    run(&mut e, "A = magic(4); t = rigidtform2d(10, [1 2]);");
    error_has(&mut e, "imwarp(A)", "not enough input arguments");
    error_has(
        &mut e,
        "imwarp(A, eye(3))",
        "a 3-by-3 matrix is not a transformation",
    );
    error_has(&mut e, "imwarp(A, 'linear')", "expected imwarp(A, tform)");
    error_has(
        &mut e,
        "imwarp(A, imref2d([4 4]))",
        "the third input must be",
    );
    error_has(
        &mut e,
        "imwarp(A, imref2d([3 4]), t)",
        "RA.ImageSize [3 4] must match the size of A",
    );
    error_has(
        &mut e,
        "imwarp(t, A)",
        "the first input must be the image A",
    );
    error_has(
        &mut e,
        "imwarp(A, imref2d([4 4]), imref2d([4 4]))",
        "third input",
    );
    error_has(
        &mut e,
        "imwarp(A, t, 'spline')",
        "unknown parameter 'spline'",
    );
    error_has(&mut e, "imwarp(A, t, 'FillValues')", "needs a value");
    error_has(
        &mut e,
        "imwarp(A, t, 'FillValues', [1 2])",
        "one value per channel (1)",
    );
    error_has(
        &mut e,
        "imwarp(A, t, 'OutputView', [4 4])",
        "OutputView must be an imref2d",
    );
    error_has(
        &mut e,
        "imwarp(A, t, 'SmoothEdges', [1 0])",
        "SmoothEdges must be a logical scalar",
    );
    error_has(
        &mut e,
        "imwarp(A, t, 'InterpolationMethod', 'box')",
        "InterpolationMethod must be",
    );
    error_has(
        &mut e,
        "imwarp(A, t, 3)",
        "expected an interpolation method or a parameter name",
    );
    error_has(
        &mut e,
        "[a, b, c] = imwarp(A, t);",
        "Too many output arguments",
    );
    error_has(&mut e, "imwarp(zeros(0, 3), t)", "A must not be empty");
    error_has(&mut e, "imwarp(1i * A, t)", "complex");
    error_has(
        &mut e,
        "imwarp(A, affinetform2d([1e9 0 0; 0 1e9 0; 0 0 1]))",
        "limit",
    );
    // The engine stays usable.
    run(&mut e, "B = imwarp(A, t);");
}

#[test]
fn a_cancellation_stops_a_large_warp() {
    use std::sync::atomic::Ordering;
    let mut e = Engine::default();
    let code = "B = imwarp(A, t, 'cubic');";
    run(&mut e, "A = rand(1500); t = rigidtform2d(10, [0 0]);");
    let start = std::time::Instant::now();
    run(&mut e, code);
    let full = start.elapsed();
    let stop = e.cancellation();
    let watcher = std::thread::spawn({
        let stop = stop.clone();
        move || {
            std::thread::sleep(full / 10);
            stop.store(true, Ordering::Relaxed);
        }
    });
    let message = error(&mut e, code);
    watcher.join().unwrap();
    stop.store(false, Ordering::Relaxed);
    assert!(message.contains("cancelled"), "{message}");
    run(&mut e, "B = imwarp(A(1:10, 1:10), t);");
}
