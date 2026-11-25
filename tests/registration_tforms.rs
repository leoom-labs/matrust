//! Image registration piece 7c1: imref2d and the 2-D geometric
//! transformation objects affinetform2d, rigidtform2d, simtform2d,
//! transltform2d (premultiply) and affine2d (postmultiply).
//!
//! Expected values are derived by hand from the formulas on the public
//! MathWorks reference pages (no MATLAB run), written next to each check:
//!
//! - premultiply: [x; y; 1] = A * [u; v; 1]; postmultiply: [x y 1] = [u v 1] * T, A = T.'
//! - rigid: A = [R t'; 0 0 1], R = [cosd(r) -sind(r); sind(r) cosd(r)]
//! - similarity: A = [s*R t'; 0 0 1]; translation: A = [1 0 tx; 0 1 ty; 0 0 1]
//! - imref2d for an m-by-n image: default XWorldLimits [0.5 n+0.5],
//!   YWorldLimits [0.5 m+0.5]; with pixel extents dx, dy the limits are
//!   dx*[0.5 n+0.5] and dy*[0.5 m+0.5]; xIntrinsic = 0.5 + (xWorld - XWorldLimits(1)) / dx.
//!
//! Two cases are the reference pages' own numbers: the worldToIntrinsic
//! example (pixel extent 0.3125: (38.44, 68.75) -> (123.008, 220) and the
//! outside point (-0.2, -1) -> (-0.64, -3.2)) and the outputLimits example
//! (vertical shear 0.33, horizontal stretch 2 of [1 240] x [1 291] gives
//! [2 480] x [1.33 370.2]).
//!
//! Tolerances: exact where the arithmetic is exact (multiples of 90 degrees,
//! dyadic values); 1e-15 relative for cosd/sind of 30 and 45 degrees; 1e-12
//! for round trips over random points.
use matrust::{Array, Engine, Matrix};

const C30: f64 = 0.866_025_403_784_438_6; // sqrt(3)/2
const S2: f64 = std::f64::consts::SQRT_2;

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

fn output(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(done) => done.output.join("\n"),
        Err(error) => panic!("{code}\n  failed: {error}"),
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

/// `expected` is row-major for readability; compared column-major.
fn check(e: &mut Engine, expr: &str, dims: &[usize], expected: &[f64], tol: f64) {
    let (d, v) = values(e, expr);
    assert_eq!(d, dims, "{expr}: size");
    let (rows, cols) = (dims[0], v.len() / dims[0].max(1));
    for i in 0..rows {
        for j in 0..cols {
            let a = v[i + j * rows];
            let b = expected[i * cols + j];
            let ok = if tol == 0. {
                a == b || a.is_nan() && b.is_nan()
            } else {
                (a - b).abs() <= tol * b.abs().max(1.) || a.is_nan() && b.is_nan()
            };
            assert!(ok, "{expr}: element ({i},{j}) is {a:e}, expected {b:e}");
        }
    }
}

fn truth(e: &mut Engine, expr: &str) -> bool {
    run(e, &format!("zz_t = double(logical({expr}));"));
    matrix(e, "zz_t").data() == [1.]
}

#[test]
fn imref2d_constructor_forms_and_derived_properties() {
    let mut e = Engine::default();
    // imref2d(): the default 2-by-2 image.
    run(&mut e, "R = imref2d();");
    check(&mut e, "R.ImageSize", &[1, 2], &[2., 2.], 0.);
    check(&mut e, "R.XWorldLimits", &[1, 2], &[0.5, 2.5], 0.);
    // imref2d([480 640]): n = 640 columns, m = 480 rows, extents 1.
    run(&mut e, "R = imref2d([480 640]);");
    check(&mut e, "R.XWorldLimits", &[1, 2], &[0.5, 640.5], 0.);
    check(&mut e, "R.YWorldLimits", &[1, 2], &[0.5, 480.5], 0.);
    check(&mut e, "R.ImageSize", &[1, 2], &[480., 640.], 0.);
    check(&mut e, "R.PixelExtentInWorldX", &[1, 1], &[1.], 0.);
    check(&mut e, "R.PixelExtentInWorldY", &[1, 1], &[1.], 0.);
    check(&mut e, "R.ImageExtentInWorldX", &[1, 1], &[640.], 0.);
    check(&mut e, "R.ImageExtentInWorldY", &[1, 1], &[480.], 0.);
    check(&mut e, "R.XIntrinsicLimits", &[1, 2], &[0.5, 640.5], 0.);
    check(&mut e, "R.YIntrinsicLimits", &[1, 2], &[0.5, 480.5], 0.);
    // A third element (size of an RGB image) is dropped from ImageSize.
    run(&mut e, "R = imref2d([480 640 3]);");
    check(&mut e, "R.ImageSize", &[1, 2], &[480., 640.], 0.);
    check(&mut e, "R.XWorldLimits", &[1, 2], &[0.5, 640.5], 0.);
    // World limits: 2-by-3 image over [0 6] x [0 4]: extents 6/3 = 2 and 4/2 = 2.
    run(&mut e, "R = imref2d([2 3], [0 6], [0 4]);");
    check(&mut e, "R.PixelExtentInWorldX", &[1, 1], &[2.], 0.);
    check(&mut e, "R.PixelExtentInWorldY", &[1, 1], &[2.], 0.);
    check(&mut e, "R.ImageExtentInWorldX", &[1, 1], &[6.], 0.);
    check(&mut e, "R.ImageExtentInWorldY", &[1, 1], &[4.], 0.);
    check(&mut e, "R.XIntrinsicLimits", &[1, 2], &[0.5, 3.5], 0.);
    check(&mut e, "R.YIntrinsicLimits", &[1, 2], &[0.5, 2.5], 0.);
    // Pixel extents dx = 2, dy = 4: limits 2*[0.5 3.5] = [1 7], 4*[0.5 2.5] = [2 10].
    run(&mut e, "R = imref2d([2 3], 2, 4);");
    check(&mut e, "R.XWorldLimits", &[1, 2], &[1., 7.], 0.);
    check(&mut e, "R.YWorldLimits", &[1, 2], &[2., 10.], 0.);
    check(&mut e, "R.PixelExtentInWorldX", &[1, 1], &[2.], 0.);
    check(&mut e, "R.PixelExtentInWorldY", &[1, 1], &[4.], 0.);
    check(&mut e, "R.ImageExtentInWorldX", &[1, 1], &[6.], 0.);
    check(&mut e, "R.ImageExtentInWorldY", &[1, 1], &[8.], 0.);
    check(&mut e, "R.XWorldLimits(2)", &[1, 1], &[7.], 0.);
}

#[test]
fn imref2d_settable_properties_have_value_semantics() {
    let mut e = Engine::default();
    run(
        &mut e,
        "R = imref2d([2 3]); R2 = R; R2.XWorldLimits = [0 6]; R2.YWorldLimits = [0 4];",
    );
    // The copy changed, the original did not.
    check(&mut e, "R.XWorldLimits", &[1, 2], &[0.5, 3.5], 0.);
    check(&mut e, "R2.XWorldLimits", &[1, 2], &[0., 6.], 0.);
    // Extents follow the limits: 6/3, 4/2.
    check(&mut e, "R2.PixelExtentInWorldX", &[1, 1], &[2.], 0.);
    check(&mut e, "R2.PixelExtentInWorldY", &[1, 1], &[2.], 0.);
    check(&mut e, "R2.ImageExtentInWorldX", &[1, 1], &[6.], 0.);
    // ImageSize keeps the world limits: [0 6] over 6 columns, [0 4] over 4 rows.
    run(&mut e, "R2.ImageSize = [4 6];");
    check(&mut e, "R2.XWorldLimits", &[1, 2], &[0., 6.], 0.);
    check(&mut e, "R2.PixelExtentInWorldX", &[1, 1], &[1.], 0.);
    check(&mut e, "R2.PixelExtentInWorldY", &[1, 1], &[1.], 0.);
    check(&mut e, "R2.XIntrinsicLimits", &[1, 2], &[0.5, 6.5], 0.);
    check(&mut e, "R2.YIntrinsicLimits", &[1, 2], &[0.5, 4.5], 0.);
    run(&mut e, "R2.ImageSize = [8 6 3];");
    check(&mut e, "R2.ImageSize", &[1, 2], &[8., 6.], 0.);
    check(&mut e, "R2.PixelExtentInWorldY", &[1, 1], &[0.5], 0.);
    // Indexed assignment into a property.
    run(&mut e, "R2.XWorldLimits(2) = 12;");
    check(&mut e, "R2.XWorldLimits", &[1, 2], &[0., 12.], 0.);
    check(&mut e, "R2.PixelExtentInWorldX", &[1, 1], &[2.], 0.);
    // Through a struct field: a changed copy as well.
    run(&mut e, "s.R = R; s.R.XWorldLimits = [1 4];");
    check(&mut e, "s.R.XWorldLimits", &[1, 2], &[1., 4.], 0.);
    check(&mut e, "R.XWorldLimits", &[1, 2], &[0.5, 3.5], 0.);
    error_has(
        &mut e,
        "R.PixelExtentInWorldX = 3;",
        "You cannot set the read-only property 'PixelExtentInWorldX' of imref2d.",
    );
    error_has(
        &mut e,
        "R.XIntrinsicLimits = [0 1];",
        "You cannot set the read-only property 'XIntrinsicLimits' of imref2d.",
    );
    error_has(
        &mut e,
        "R.XWorldLimits = [3 1];",
        "imref2d: XWorldLimits must be finite and increasing",
    );
    error_has(
        &mut e,
        "R.ImageSize = [0 2];",
        "imref2d: imageSize must contain positive integers",
    );
    error_has(
        &mut e,
        "R.Foo = 1;",
        "Unrecognized property 'Foo' for class 'imref2d'.",
    );
    // The failed assignments left R as it was.
    check(&mut e, "R.XWorldLimits", &[1, 2], &[0.5, 3.5], 0.);
}

#[test]
fn imref2d_methods_including_points_outside() {
    let mut e = Engine::default();
    // Published example: extent 0.3125, so XWorldLimits(1) = 0.15625 and
    // xi = 0.5 + (38.44 - 0.15625) / 0.3125 = 123.008, yi = 0.5 + 68.59375/0.3125 = 220;
    // outside: 0.5 + (-0.35625)/0.3125 = -0.64, 0.5 + (-1.15625)/0.3125 = -3.2.
    run(
        &mut e,
        "R = imref2d([512 512], 0.3125, 0.3125); [xi, yi] = worldToIntrinsic(R, [38.44 39.44 38.44 -0.2], [68.75 68.75 75.75 -1]);",
    );
    check(
        &mut e,
        "xi",
        &[1, 4],
        &[123.008, 126.208, 123.008, -0.64],
        1e-12,
    );
    check(&mut e, "yi", &[1, 4], &[220., 220., 242.4, -3.2], 1e-12);
    // intrinsicToWorld of imref2d([2 3], 2, 4): x = 1 + (xi - 0.5)*2, y = 2 + (yi - 0.5)*4.
    run(
        &mut e,
        "R = imref2d([2 3], 2, 4); [xw, yw] = intrinsicToWorld(R, [1 3; 0 10], [1 2; -1 0.5]);",
    );
    check(&mut e, "xw", &[2, 2], &[2., 6., 0., 20.], 0.);
    check(&mut e, "yw", &[2, 2], &[4., 8., -4., 2.], 0.);
    // Round trip through intrinsic coordinates.
    run(
        &mut e,
        "[a, b] = worldToIntrinsic(R, xw, yw); [c, d] = intrinsicToWorld(R, a, b);",
    );
    check(&mut e, "a", &[2, 2], &[1., 3., 0., 10.], 1e-15);
    check(&mut e, "d", &[2, 2], &[4., 8., -4., 2.], 1e-15);
    // worldToSubscript on the default 2-by-3 grid: x in [0.5 3.5], y in [0.5 2.5].
    // (1, 2) -> row 2, col 1; (3.4, 1) -> row 1, col round(3.4) = 3; (1.5, 1.5)
    // rounds half away from zero -> (2, 2); the far corner (3.5, 2.5) stays in
    // the last pixel (2, 3); (0.5, 0.5) -> (1, 1); outside points give NaN.
    run(
        &mut e,
        "R = imref2d([2 3]); [I, J] = worldToSubscript(R, [1 3.4 1.5 3.5 0.5 9 1 NaN], [2 1 1.5 2.5 0.5 1 0.4 1]);",
    );
    let nan = f64::NAN;
    check(
        &mut e,
        "I",
        &[1, 8],
        &[2., 1., 2., 2., 1., nan, nan, nan],
        0.,
    );
    check(
        &mut e,
        "J",
        &[1, 8],
        &[1., 3., 2., 3., 1., nan, nan, nan],
        0.,
    );
    run(&mut e, "I1 = worldToSubscript(R, 2, 2);");
    check(&mut e, "I1", &[1, 1], &[2.], 0.);
    // contains: borders count as inside.
    check(
        &mut e,
        "contains(R, [0.5 3.5 3.6 2 NaN], [0.5 2.5 1 0.4 1])",
        &[1, 5],
        &[1., 1., 0., 0., 0.],
        0.,
    );
    assert!(truth(&mut e, "islogical(contains(R, 1, 1))"));
    check(
        &mut e,
        "contains(R, [1; 2], [1; 1])",
        &[2, 1],
        &[1., 1.],
        0.,
    );
    // The string function is unchanged.
    assert!(truth(&mut e, "contains(\"abc\", \"b\")"));
    // sizesMatch compares the first two dimensions only.
    assert!(truth(&mut e, "sizesMatch(R, zeros(2, 3))"));
    assert!(truth(&mut e, "sizesMatch(R, zeros(2, 3, 3))"));
    assert!(!truth(&mut e, "sizesMatch(R, zeros(3, 2))"));
    // Single input gives single output; dot-call forms.
    assert!(truth(
        &mut e,
        "isa(worldToIntrinsic(R, single(1), single(1)), 'single')"
    ));
    run(
        &mut e,
        "[p, q] = R.worldToIntrinsic(2, 3); r = R.contains(9, 9);",
    );
    check(&mut e, "[p q r]", &[1, 3], &[2., 3., 0.], 0.);
    error_has(
        &mut e,
        "worldToIntrinsic(R, [1 2], [1 2 3])",
        "worldToIntrinsic: the two coordinate arrays must be the same size",
    );
}

#[test]
fn transformation_constructor_forms() {
    let mut e = Engine::default();
    let id3 = [1., 0., 0., 0., 1., 0., 0., 0., 1.];
    for class in [
        "affinetform2d",
        "rigidtform2d",
        "simtform2d",
        "transltform2d",
        "affine2d",
    ] {
        run(&mut e, &format!("t = {class}();"));
        let property = if class == "affine2d" { "T" } else { "A" };
        check(&mut e, &format!("t.{property}"), &[3, 3], &id3, 0.);
        check(&mut e, "t.Dimensionality", &[1, 1], &[2.], 0.);
        assert!(truth(&mut e, &format!("strcmp(class(t), '{class}')")));
    }
    // rigidtform2d(t): translation only.
    run(&mut e, "t = rigidtform2d([3 4]);");
    check(
        &mut e,
        "t.A",
        &[3, 3],
        &[1., 0., 3., 0., 1., 4., 0., 0., 1.],
        0.,
    );
    check(&mut e, "t.RotationAngle", &[1, 1], &[0.], 0.);
    // rigidtform2d(30, [10 20.5]): R = [c -1/2; 1/2 c], c = sqrt(3)/2.
    run(&mut e, "t = rigidtform2d(30, [10 20.5]);");
    check(&mut e, "t.RotationAngle", &[1, 1], &[30.], 0.);
    check(&mut e, "t.Translation", &[1, 2], &[10., 20.5], 0.);
    check(&mut e, "t.R", &[2, 2], &[C30, -0.5, 0.5, C30], 1e-15);
    check(
        &mut e,
        "t.A",
        &[3, 3],
        &[C30, -0.5, 10., 0.5, C30, 20.5, 0., 0., 1.],
        1e-15,
    );
    // rigidtform2d(R, t) with the quarter turn: angle 90.
    run(&mut e, "t = rigidtform2d([0 -1; 1 0], [1 2]);");
    check(&mut e, "t.RotationAngle", &[1, 1], &[90.], 0.);
    check(
        &mut e,
        "t.A",
        &[3, 3],
        &[0., -1., 1., 1., 0., 2., 0., 0., 1.],
        0.,
    );
    // rigidtform2d(A), 3-by-3 and 2-by-3: a half turn is angle -180 in [-180, 180).
    run(&mut e, "t = rigidtform2d([-1 0 5; 0 -1 6; 0 0 1]);");
    check(&mut e, "t.RotationAngle", &[1, 1], &[-180.], 0.);
    check(&mut e, "t.Translation", &[1, 2], &[5., 6.], 0.);
    run(&mut e, "t = rigidtform2d([0 1 5; -1 0 6]);");
    check(&mut e, "t.RotationAngle", &[1, 1], &[-90.], 0.);
    // Angles wrap into [-180, 180).
    run(
        &mut e,
        "t = rigidtform2d(270, [0 0]); u = rigidtform2d(180, [0 0]);",
    );
    check(
        &mut e,
        "[t.RotationAngle u.RotationAngle]",
        &[1, 2],
        &[-90., -180.],
        0.,
    );
    check(&mut e, "t.R", &[2, 2], &[0., 1., -1., 0.], 0.);
    // rigidtform2d(tformIn) from a rigid affinetform2d.
    run(
        &mut e,
        "t = rigidtform2d(affinetform2d([0 -1 2; 1 0 3; 0 0 1]));",
    );
    check(
        &mut e,
        "[t.RotationAngle t.Translation]",
        &[1, 3],
        &[90., 2., 3.],
        0.,
    );
    // simtform2d(2, 30, [1 2]): A = [2c -1 1; 1 2c 2; 0 0 1], R without the scale.
    run(&mut e, "t = simtform2d(2, 30, [1 2]);");
    check(&mut e, "t.Scale", &[1, 1], &[2.], 0.);
    check(&mut e, "t.RotationAngle", &[1, 1], &[30.], 0.);
    check(&mut e, "t.R", &[2, 2], &[C30, -0.5, 0.5, C30], 1e-15);
    check(
        &mut e,
        "t.A",
        &[3, 3],
        &[2. * C30, -1., 1., 1., 2. * C30, 2., 0., 0., 1.],
        1e-15,
    );
    run(&mut e, "t = simtform2d(2, [0 -1; 1 0], [1 2]);");
    check(
        &mut e,
        "t.A",
        &[3, 3],
        &[0., -2., 1., 2., 0., 2., 0., 0., 1.],
        0.,
    );
    // simtform2d(A): scale = sqrt((0 + 9 + 9 + 0)/2) = 3, angle 90.
    run(&mut e, "t = simtform2d([0 -3 1; 3 0 2; 0 0 1]);");
    check(&mut e, "[t.Scale t.RotationAngle]", &[1, 2], &[3., 90.], 0.);
    check(&mut e, "t.R", &[2, 2], &[0., -1., 1., 0.], 0.);
    // transltform2d forms.
    for code in [
        "t = transltform2d(3, -1);",
        "t = transltform2d([3 -1]);",
        "t = transltform2d([1 0 3; 0 1 -1; 0 0 1]);",
        "t = transltform2d([1 0 3; 0 1 -1]);",
        "t = transltform2d(affine2d([1 0 0; 0 1 0; 3 -1 1]));",
    ] {
        run(&mut e, code);
        check(&mut e, "t.Translation", &[1, 2], &[3., -1.], 0.);
        check(
            &mut e,
            "t.A",
            &[3, 3],
            &[1., 0., 3., 0., 1., -1., 0., 0., 1.],
            0.,
        );
    }
    // affinetform2d forms: 3-by-3, 2-by-3, from rigid, from affine2d (A = T.').
    run(&mut e, "t = affinetform2d([2 1 3; 4 5 6]);");
    check(
        &mut e,
        "t.A",
        &[3, 3],
        &[2., 1., 3., 4., 5., 6., 0., 0., 1.],
        0.,
    );
    run(&mut e, "t = affinetform2d(rigidtform2d(90, [1 2]));");
    check(
        &mut e,
        "t.A",
        &[3, 3],
        &[0., -1., 1., 1., 0., 2., 0., 0., 1.],
        0.,
    );
    run(
        &mut e,
        "t = affinetform2d(affine2d([2 4 0; 1 5 0; 3 6 1]));",
    );
    check(
        &mut e,
        "t.A",
        &[3, 3],
        &[2., 1., 3., 4., 5., 6., 0., 0., 1.],
        0.,
    );
    run(&mut e, "t = affine2d([2 4 0; 1 5 0; 3 6 1]);");
    check(
        &mut e,
        "t.T",
        &[3, 3],
        &[2., 4., 0., 1., 5., 0., 3., 6., 1.],
        0.,
    );
}

#[test]
fn setting_properties_keeps_a_consistent() {
    let mut e = Engine::default();
    run(&mut e, "t = rigidtform2d(30, [10 20]); t0 = t;");
    // RotationAngle 90: A block = [0 -1; 1 0], translation kept.
    run(&mut e, "t.RotationAngle = 90;");
    check(
        &mut e,
        "t.A",
        &[3, 3],
        &[0., -1., 10., 1., 0., 20., 0., 0., 1.],
        0.,
    );
    check(&mut e, "t0.RotationAngle", &[1, 1], &[30.], 0.);
    run(&mut e, "t.Translation = [5 6];");
    check(&mut e, "t.A(1:2, 3)", &[2, 1], &[5., 6.], 0.);
    run(&mut e, "t.Translation(2) = 9;");
    check(&mut e, "t.Translation", &[1, 2], &[5., 9.], 0.);
    // R = [0 1; -1 0] is the -90 degree rotation.
    run(&mut e, "t.R = [0 1; -1 0];");
    check(&mut e, "t.RotationAngle", &[1, 1], &[-90.], 0.);
    check(
        &mut e,
        "t.A",
        &[3, 3],
        &[0., 1., 5., -1., 0., 9., 0., 0., 1.],
        0.,
    );
    run(&mut e, "t.A = [-1 0 1; 0 -1 2; 0 0 1];");
    check(
        &mut e,
        "[t.RotationAngle t.Translation]",
        &[1, 3],
        &[-180., 1., 2.],
        0.,
    );
    // The untouched copy still has its old A.
    check(
        &mut e,
        "t0.A",
        &[3, 3],
        &[C30, -0.5, 10., 0.5, C30, 20., 0., 0., 1.],
        1e-15,
    );
    // Similarity: Scale 3 scales the 2-by-2 block of A.
    run(&mut e, "s = simtform2d(2, 90, [1 2]); s.Scale = 3;");
    check(
        &mut e,
        "s.A",
        &[3, 3],
        &[0., -3., 1., 3., 0., 2., 0., 0., 1.],
        0.,
    );
    run(&mut e, "s.RotationAngle = 0;");
    check(
        &mut e,
        "s.A",
        &[3, 3],
        &[3., 0., 1., 0., 3., 2., 0., 0., 1.],
        0.,
    );
    run(&mut e, "s.A = [0 -2 0; 2 0 0; 0 0 1];");
    check(&mut e, "[s.Scale s.RotationAngle]", &[1, 2], &[2., 90.], 0.);
    run(&mut e, "s.R = [1 0; 0 1];");
    check(
        &mut e,
        "s.A",
        &[3, 3],
        &[2., 0., 0., 0., 2., 0., 0., 0., 1.],
        0.,
    );
    // Translation and affine objects.
    run(&mut e, "u = transltform2d(1, 2); u.Translation = [7 8];");
    check(&mut e, "u.A(1:2, 3)", &[2, 1], &[7., 8.], 0.);
    run(&mut e, "u.A = [1 0 -1; 0 1 -2; 0 0 1];");
    check(&mut e, "u.Translation", &[1, 2], &[-1., -2.], 0.);
    run(
        &mut e,
        "a = affinetform2d(); a.A = [1 2 3; 4 5 6]; a.A(1, 2) = 7;",
    );
    check(
        &mut e,
        "a.A",
        &[3, 3],
        &[1., 7., 3., 4., 5., 6., 0., 0., 1.],
        0.,
    );
    run(&mut e, "b = affine2d(); b.T = [1 0 0; 2 1 0; 3 4 1];");
    check(
        &mut e,
        "b.T",
        &[3, 3],
        &[1., 0., 0., 2., 1., 0., 3., 4., 1.],
        0.,
    );
    // Read-only and invalid values leave the object unchanged.
    error_has(
        &mut e,
        "t.Dimensionality = 3;",
        "You cannot set the read-only property 'Dimensionality' of rigidtform2d.",
    );
    error_has(
        &mut e,
        "t.R = [1 1; 0 1];",
        "rigidtform2d: R must be a rotation matrix",
    );
    error_has(
        &mut e,
        "s.Scale = -1;",
        "simtform2d: the scale must be positive",
    );
    error_has(
        &mut e,
        "u.A = [0 -1 0; 1 0 0; 0 0 1];",
        "transltform2d: the matrix is not a translation",
    );
    error_has(
        &mut e,
        "t.Scale = 2;",
        "Unrecognized property 'Scale' for class 'rigidtform2d'.",
    );
    check(&mut e, "t.RotationAngle", &[1, 1], &[-180.], 0.);
    error_has(
        &mut e,
        "t.NoSuch",
        "Unrecognized method, property, or field 'NoSuch' for class 'rigidtform2d'.",
    );
}

#[test]
fn affine2d_and_affinetform2d_conventions_map_the_same_points() {
    let mut e = Engine::default();
    // T postmultiplies row points; A = T.' premultiplies column points.
    // (1, 1): [1 1 1] * T = [2 - 1 + 4, 0.5 + 3 + 5, 1] = [5, 8.5, 1].
    run(
        &mut e,
        "T = [2 0.5 0; -1 3 0; 4 5 1]; t1 = affine2d(T); t2 = affinetform2d(T.');
         [x1, y1] = transformPointsForward(t1, 1, 1);",
    );
    check(&mut e, "[x1 y1]", &[1, 2], &[5., 8.5], 0.);
    run(
        &mut e,
        "rng(3); u = 200*rand(40, 1) - 100; v = 200*rand(40, 1) - 100;
         [x1, y1] = transformPointsForward(t1, u, v); [x2, y2] = transformPointsForward(t2, u, v);
         p = [u v 1 + 0*u] * T;
         [a1, b1] = transformPointsInverse(t1, x1, y1); [a2, b2] = transformPointsInverse(t2, x1, y1);",
    );
    let (_, x1) = values(&mut e, "x1");
    let (_, x2) = values(&mut e, "x2");
    let (_, y1) = values(&mut e, "y1");
    let (_, y2) = values(&mut e, "y2");
    assert_eq!(x1, x2);
    assert_eq!(y1, y2);
    let (_, p) = values(&mut e, "p");
    for i in 0..40 {
        assert!((x1[i] - p[i]).abs() <= 1e-12 * p[i].abs().max(1.));
        assert!((y1[i] - p[40 + i]).abs() <= 1e-12 * p[40 + i].abs().max(1.));
    }
    let (_, a1) = values(&mut e, "a1");
    let (_, a2) = values(&mut e, "a2");
    assert_eq!(a1, a2);
    // affinetform2d(affine2d) is the transposed matrix.
    assert!(truth(&mut e, "isequal(affinetform2d(t1).A, T.')"));
    assert!(truth(&mut e, "isequal(affinetform2d(t1), t2)"));
}

#[test]
fn forward_inverse_round_trip_on_random_points() {
    let mut e = Engine::default();
    run(
        &mut e,
        "rng(11); x = 200*rand(25, 4) - 100; y = 200*rand(25, 4) - 100;",
    );
    let (_, x) = values(&mut e, "x");
    let (_, y) = values(&mut e, "y");
    for make in [
        "affinetform2d([1.5 -0.7 12; 0.3 2.2 -4; 0 0 1])",
        "rigidtform2d(37.5, [-12 8.25])",
        "simtform2d(0.37, -123, [5 -6])",
        "transltform2d(-3.5, 1e3)",
        "affine2d([0.9 0.2 0; -1.1 0.4 0; 7 8 1])",
    ] {
        run(
            &mut e,
            &format!(
                "t = {make}; [u, v] = transformPointsForward(t, x, y); [xr, yr] = transformPointsInverse(t, u, v);
                 XY = transformPointsInverse(t, transformPointsForward(t, [x(:) y(:)]));"
            ),
        );
        let (dims, xr) = values(&mut e, "xr");
        assert_eq!(dims, [25, 4], "{make}: shape kept");
        let (_, yr) = values(&mut e, "yr");
        let (_, xy) = values(&mut e, "XY");
        for i in 0..100 {
            assert!(
                (xr[i] - x[i]).abs() <= 1e-12 * x[i].abs().max(1.),
                "{make} x {i}"
            );
            assert!(
                (yr[i] - y[i]).abs() <= 1e-12 * y[i].abs().max(1.),
                "{make} y {i}"
            );
            assert!(
                (xy[i] - x[i]).abs() <= 1e-12 * x[i].abs().max(1.),
                "{make} XY {i}"
            );
            assert!((xy[100 + i] - y[i]).abs() <= 1e-12 * y[i].abs().max(1.));
        }
    }
    // The n-by-2 form and the two-array form agree; single stays single.
    run(
        &mut e,
        "t = rigidtform2d(90, [1 2]); P = transformPointsForward(t, [1 0; 0 1; 2 3]);",
    );
    // (1,0) -> (0*1 - 1*0 + 1, 1*1 + 0 + 2) = (1, 3); (0,1) -> (0, 2); (2,3) -> (-2, 4).
    check(&mut e, "P", &[3, 2], &[1., 3., 0., 2., -2., 4.], 0.);
    assert!(truth(
        &mut e,
        "isa(transformPointsForward(t, single(1), 2), 'single')"
    ));
    run(
        &mut e,
        "[a, b] = t.transformPointsForward(1, 0); c = t.transformPointsInverse([1 3]);",
    );
    check(&mut e, "[a b c]", &[1, 4], &[1., 3., 1., 0.], 0.);
    run(&mut e, "a1 = transformPointsForward(t, 1, 0);");
    check(&mut e, "a1", &[1, 1], &[1.], 0.);
}

#[test]
fn invert_keeps_the_class_and_inverts_the_map() {
    let mut e = Engine::default();
    // Rigid: angle -30, translation -R'*t.
    run(&mut e, "t = rigidtform2d(30, [10 20]); i = invert(t);");
    assert!(truth(&mut e, "isa(i, 'rigidtform2d')"));
    check(&mut e, "i.RotationAngle", &[1, 1], &[-30.], 0.);
    // -R'*t = -[c*10 + 0.5*20; -0.5*10 + c*20] = [-(10c + 10), 5 - 20c].
    check(
        &mut e,
        "i.Translation",
        &[1, 2],
        &[-(10. * C30 + 10.), 5. - 20. * C30],
        1e-15,
    );
    let id3 = [1., 0., 0., 0., 1., 0., 0., 0., 1.];
    check(&mut e, "i.A * t.A", &[3, 3], &id3, 1e-15);
    // Similarity: scale 1/2.
    run(&mut e, "t = simtform2d(2, 90, [1 2]); i = invert(t);");
    check(
        &mut e,
        "[i.Scale i.RotationAngle]",
        &[1, 2],
        &[0.5, -90.],
        0.,
    );
    check(&mut e, "i.A * t.A", &[3, 3], &id3, 0.);
    run(&mut e, "i = invert(transltform2d(3, 4));");
    check(&mut e, "i.Translation", &[1, 2], &[-3., -4.], 0.);
    // Affine: inverse of [2 0 1; 0 4 2] is [1/2 0 -1/2; 0 1/4 -1/2].
    run(&mut e, "i = invert(affinetform2d([2 0 1; 0 4 2; 0 0 1]));");
    check(
        &mut e,
        "i.A",
        &[3, 3],
        &[0.5, 0., -0.5, 0., 0.25, -0.5, 0., 0., 1.],
        0.,
    );
    run(
        &mut e,
        "T = [1 2 0; 3 4 0; 5 6 1]; i = invert(affine2d(T));",
    );
    assert!(truth(&mut e, "isa(i, 'affine2d')"));
    check(&mut e, "T * i.T", &[3, 3], &id3, 1e-14);
    run(&mut e, "j = invert(rigidtform2d());");
    check(
        &mut e,
        "[j.RotationAngle j.Translation]",
        &[1, 3],
        &[0., 0., 0.],
        0.,
    );
    run(&mut e, "k = t.invert();");
    check(&mut e, "k.Scale", &[1, 1], &[0.5], 0.);
}

#[test]
fn output_limits_of_rotations_and_shears() {
    let mut e = Engine::default();
    // 45-degree rotation of the square [0.5 100.5]^2: u = c(x - y), v = c(x + y)
    // with c = sqrt(2)/2; corners give u in [-100c, 100c] = [-50 sqrt2, 50 sqrt2]
    // and v in [c, 201c] = [sqrt2/2, 100.5 sqrt2].
    run(
        &mut e,
        "[xl, yl] = outputLimits(rigidtform2d(45, [0 0]), [0.5 100.5], [0.5 100.5]);",
    );
    check(&mut e, "xl", &[1, 2], &[-50. * S2, 50. * S2], 1e-14);
    check(&mut e, "yl", &[1, 2], &[S2 / 2., 100.5 * S2], 1e-14);
    // Reference page example: horizontal stretch 2, vertical shear 0.33 of
    // [1 240] x [1 291]: x' = 2x in [2 480], y' = 0.33x + y in [1.33, 79.2 + 291].
    run(
        &mut e,
        "[xl, yl] = outputLimits(affinetform2d([2 0 0; 0.33 1 0; 0 0 1]), [1 240], [1 291]);",
    );
    check(&mut e, "xl", &[1, 2], &[2., 480.], 1e-14);
    check(&mut e, "yl", &[1, 2], &[1.33, 370.2], 1e-14);
    // affine2d with the same map as rows: T = A.'.
    run(
        &mut e,
        "[xl2, yl2] = outputLimits(affine2d([2 0.33 0; 0 1 0; 0 0 1]), [1 240], [1 291]);",
    );
    assert!(truth(&mut e, "isequal(xl, xl2) && isequal(yl, yl2)"));
    run(
        &mut e,
        "xl3 = outputLimits(transltform2d(1, 2), [0 1], [0 1]);",
    );
    check(&mut e, "xl3", &[1, 2], &[1., 2.], 0.);
}

#[test]
fn rigid_similarity_and_translation_tests() {
    let mut e = Engine::default();
    // [rigid, similarity, translation] for each matrix.
    let cases: [(&str, [bool; 3]); 7] = [
        ("[0 -1 5; 1 0 2; 0 0 1]", [true, true, false]),
        ("[0 -2 5; 2 0 2; 0 0 1]", [false, true, false]),
        ("[1 0 0; 0 -1 0; 0 0 1]", [false, true, false]),
        ("[1 0.5 0; 0 1 0; 0 0 1]", [false, false, false]),
        ("[2 0 0; 0 3 0; 0 0 1]", [false, false, false]),
        ("[1 0 4; 0 1 5; 0 0 1]", [true, true, true]),
        ("[1+1e-13 0 4; 0 1 5; 0 0 1]", [true, true, true]),
    ];
    for (a, expected) in cases {
        for make in [format!("affinetform2d({a})"), format!("affine2d(({a}).')")] {
            run(&mut e, &format!("t = {make};"));
            let got = [
                truth(&mut e, "isRigid(t)"),
                truth(&mut e, "isSimilarity(t)"),
                truth(&mut e, "isTranslation(t)"),
            ];
            assert_eq!(got, expected, "{make}");
        }
    }
    assert!(truth(&mut e, "islogical(isRigid(affine2d()))"));
    // A 30-degree rotation built from cosd/sind is rigid.
    assert!(truth(
        &mut e,
        "isRigid(affinetform2d(rigidtform2d(30, [1 2]).A))"
    ));
    // Only affine2d and affinetform2d have these methods.
    error_has(
        &mut e,
        "isRigid(rigidtform2d())",
        "Undefined function 'isRigid' for input arguments of type 'rigidtform2d'.",
    );
}

#[test]
fn validation_error_texts() {
    let mut e = Engine::default();
    for (code, part) in [
        (
            "affinetform2d([1 2 3; 4 5 6; 7 8 9])",
            "affinetform2d: the last row of the matrix must be [0 0 1]",
        ),
        (
            "affinetform2d([1 2; 3 4])",
            "affinetform2d: A must be a 3-by-3 matrix (or its 2-by-3 top rows)",
        ),
        (
            "affinetform2d([1 2 0; 2 4 0; 0 0 1])",
            "affinetform2d: A must be invertible (nonsingular)",
        ),
        (
            "affinetform2d([NaN 0 0; 0 1 0; 0 0 1])",
            "affinetform2d: the values must be finite",
        ),
        (
            "affinetform2d('abc')",
            "affinetform2d: each input must be numeric, not char",
        ),
        (
            "affinetform2d(imref2d([2 2]))",
            "affinetform2d: cannot make a affinetform2d from a imref2d object",
        ),
        (
            "rigidtform2d([1 1; 0 1], [0 0])",
            "rigidtform2d: R must be a rotation matrix",
        ),
        (
            "rigidtform2d([-1 0; 0 1], [0 0])",
            "rigidtform2d: R must be a rotation matrix",
        ),
        (
            "rigidtform2d([2 0 0; 0 2 0; 0 0 1])",
            "rigidtform2d: the matrix is not a rigid transformation",
        ),
        (
            "rigidtform2d(affinetform2d([2 0 0; 0 2 0; 0 0 1]))",
            "rigidtform2d: the matrix is not a rigid transformation",
        ),
        (
            "rigidtform2d(30, [1 2 3])",
            "rigidtform2d: the translation t must be a 2-element numeric vector",
        ),
        (
            "rigidtform2d(1, 2, 3)",
            "rigidtform2d: expected rigidtform2d(), rigidtform2d(t)",
        ),
        (
            "simtform2d(-1, 0, [0 0])",
            "simtform2d: the scale must be positive",
        ),
        (
            "simtform2d([1 0 0; 0 -1 0; 0 0 1])",
            "simtform2d: the matrix is not a similarity transformation",
        ),
        (
            "transltform2d([0 -1 0; 1 0 0; 0 0 1])",
            "transltform2d: the matrix is not a translation",
        ),
        (
            "transltform2d(1, [2 3])",
            "transltform2d: ty must be a numeric scalar",
        ),
        (
            "affine2d([1 0 1; 0 1 0; 0 0 1])",
            "affine2d: the last column of T must be [0; 0; 1]",
        ),
        (
            "affine2d([1 0 0; 0 1 0])",
            "affine2d: T must be a 3-by-3 matrix",
        ),
        (
            "imref2d([0 3])",
            "imref2d: imageSize must contain positive integers",
        ),
        (
            "imref2d([2.5 3])",
            "imref2d: imageSize must contain positive integers",
        ),
        (
            "imref2d(5)",
            "imref2d: imageSize must be a 2-element vector [rows cols]",
        ),
        (
            "imref2d([2 3], [1 0], [0 1])",
            "imref2d: xWorldLimits must be finite and increasing",
        ),
        (
            "imref2d([2 3], -1, 1)",
            "imref2d: the pixel extents must be positive finite numbers",
        ),
        (
            "imref2d([2 3], 1, [0 1])",
            "imref2d: expected pixel extents (two positive scalars) or world limits",
        ),
        (
            "imref2d([2 3], 1)",
            "imref2d: expected imref2d(), imref2d(imageSize)",
        ),
        (
            "transformPointsForward(rigidtform2d(), [1 2], [1 2 3])",
            "transformPointsForward: the two coordinate arrays must be the same size",
        ),
        (
            "transformPointsForward(rigidtform2d(), [1 2 3])",
            "transformPointsForward: a single point argument must be an n-by-2 matrix",
        ),
        (
            "transformPointsInverse(rigidtform2d(), int8(1), int8(2))",
            "transformPointsInverse: point coordinates must be single or double, not int8",
        ),
        (
            "outputLimits(rigidtform2d(), [0 1 2], [0 1])",
            "outputLimits: xLimitsIn must be a 2-element numeric vector",
        ),
        (
            "invert(5)",
            "Undefined function 'invert' for input arguments of type 'double'.",
        ),
        (
            "worldToIntrinsic(rigidtform2d(), 1, 1)",
            "Undefined function 'worldToIntrinsic' for input arguments of type 'rigidtform2d'.",
        ),
        (
            "transformPointsForward(imref2d([2 2]), 1, 1)",
            "Undefined function 'transformPointsForward' for input arguments of type 'imref2d'.",
        ),
    ] {
        error_has(&mut e, code, part);
    }
    error_has(
        &mut e,
        "[a, b] = invert(rigidtform2d());",
        "Too many output arguments.",
    );
    error_has(
        &mut e,
        "[a, b] = transformPointsForward(rigidtform2d(), [1 2]);",
        "Too many output arguments.",
    );
    error_has(
        &mut e,
        "t = rigidtform2d(); t == t",
        "Undefined function 'eq' for input arguments of type 'rigidtform2d'.",
    );
}

#[test]
fn class_queries_properties_methods_and_equality() {
    let mut e = Engine::default();
    run(
        &mut e,
        "t = rigidtform2d(30, [1 2]); R = imref2d([2 3]); a = affine2d();",
    );
    assert!(truth(&mut e, "strcmp(class(t), 'rigidtform2d')"));
    assert!(truth(&mut e, "isa(t, 'rigidtform2d')"));
    assert!(!truth(&mut e, "isa(t, 'affinetform2d')"));
    assert!(!truth(&mut e, "isa(t, 'handle')"));
    assert!(truth(
        &mut e,
        "isobject(t) && ~isstruct(t) && ~isnumeric(R)"
    ));
    assert!(truth(&mut e, "isequal(size(t), [1 1]) && numel(R) == 1"));
    assert!(truth(
        &mut e,
        "isequal(properties(t), {'Dimensionality'; 'RotationAngle'; 'Translation'; 'R'; 'A'})"
    ));
    assert!(truth(
        &mut e,
        "isequal(properties(a), {'T'; 'Dimensionality'})"
    ));
    assert!(truth(
        &mut e,
        "isequal(fieldnames(R), {'XWorldLimits'; 'YWorldLimits'; 'ImageSize'; 'PixelExtentInWorldX'; 'PixelExtentInWorldY'; 'ImageExtentInWorldX'; 'ImageExtentInWorldY'; 'XIntrinsicLimits'; 'YIntrinsicLimits'})"
    ));
    assert!(truth(
        &mut e,
        "isequal(methods(t), {'invert'; 'outputLimits'; 'transformPointsForward'; 'transformPointsInverse'})"
    ));
    assert!(truth(
        &mut e,
        "isequal(methods(a), {'invert'; 'isRigid'; 'isSimilarity'; 'isTranslation'; 'outputLimits'; 'transformPointsForward'; 'transformPointsInverse'})"
    ));
    assert!(truth(
        &mut e,
        "isequal(methods(R), {'contains'; 'intrinsicToWorld'; 'sizesMatch'; 'worldToIntrinsic'; 'worldToSubscript'})"
    ));
    assert!(truth(
        &mut e,
        "isprop(t, 'RotationAngle') && ~isprop(t, 'Scale')"
    ));
    assert!(truth(
        &mut e,
        "ismethod(R, 'contains') && ~ismethod(R, 'invert')"
    ));
    // Value equality: same class and properties.
    assert!(truth(&mut e, "isequal(t, rigidtform2d(30, [1 2]))"));
    assert!(!truth(&mut e, "isequal(t, rigidtform2d(30, [1 3]))"));
    assert!(!truth(&mut e, "isequal(affinetform2d(), rigidtform2d())"));
    assert!(truth(&mut e, "isequal(R, imref2d([2 3]), imref2d([2 3]))"));
    assert!(!truth(&mut e, "isequal(R, 5)"));
    // Listings at statement level.
    let text = output(&mut e, "properties(t)");
    assert!(
        text.contains("Properties for class rigidtform2d:") && text.contains("    RotationAngle\n"),
        "{text:?}"
    );
    let text = output(&mut e, "methods(R)");
    assert!(
        text.contains("Methods for class imref2d:") && text.contains("    worldToSubscript\n"),
        "{text:?}"
    );
    // Objects inside a cell show as [1x1 class].
    let text = output(&mut e, "c = {t, R}");
    assert!(
        text.contains("[1x1 rigidtform2d]") && text.contains("[1x1 imref2d]"),
        "{text:?}"
    );
}

#[test]
fn display_text_of_each_class() {
    let mut e = Engine::default();
    // "  <class> with properties:", a blank line, then the properties as the
    // struct display aligns them; matrices as [rows x cols class].
    let cases = [
        (
            "t = rigidtform2d(30, [10 20.5])",
            "  rigidtform2d with properties:\n\n    Dimensionality: 2\n     RotationAngle: 30\n       Translation: [10 20.5000]\n                 R: [2x2 double]\n                 A: [3x3 double]\n",
        ),
        (
            "s = simtform2d(3, 30, [10 20.5])",
            "  simtform2d with properties:\n\n    Dimensionality: 2\n             Scale: 3\n     RotationAngle: 30\n       Translation: [10 20.5000]\n                 R: [2x2 double]\n                 A: [3x3 double]\n",
        ),
        (
            "u = transltform2d([10 20.5])",
            "  transltform2d with properties:\n\n    Dimensionality: 2\n       Translation: [10 20.5000]\n                 A: [3x3 double]\n",
        ),
        (
            "f = affinetform2d()",
            "  affinetform2d with properties:\n\n    Dimensionality: 2\n                 A: [3x3 double]\n",
        ),
        (
            "a = affine2d()",
            "  affine2d with properties:\n\n                 T: [3x3 double]\n    Dimensionality: 2\n",
        ),
        (
            "R = imref2d([256 256])",
            "  imref2d with properties:\n\n           XWorldLimits: [0.5000 256.5000]\n           YWorldLimits: [0.5000 256.5000]\n              ImageSize: [256 256]\n    PixelExtentInWorldX: 1\n    PixelExtentInWorldY: 1\n    ImageExtentInWorldX: 256\n    ImageExtentInWorldY: 256\n       XIntrinsicLimits: [0.5000 256.5000]\n       YIntrinsicLimits: [0.5000 256.5000]\n",
        ),
    ];
    for (code, body) in cases {
        let body = body.trim_end_matches('\n');
        let name = code.split(' ').next().unwrap();
        let text = output(&mut e, code);
        assert!(
            text.contains(&format!("{name} = ")) && text.contains(body),
            "{code}:\n{text}"
        );
        let shown = output(&mut e, &format!("disp({name})"));
        assert!(shown.contains(body), "disp({name}):\n{shown}");
    }
    // Setting a property without a semicolon shows the changed copy.
    let text = output(&mut e, "t.RotationAngle = -45");
    assert!(text.contains("     RotationAngle: -45\n"), "{text}");
}
