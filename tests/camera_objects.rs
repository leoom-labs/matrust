//! Camera calibration piece 7d1: cameraIntrinsics, cameraParameters,
//! generateCheckerboardPoints, worldToImage and the rotation-vector
//! conversions.
//!
//! Expected values: the public MathWorks reference pages' own examples
//! (rotationVectorToMatrix / rotvec2mat3d of pi/2*[0 0 1]), hand values from
//! the documented formulas (K = [fx s cx; 0 fy cy; 0 0 1], IntrinsicMatrix =
//! K.', checkerboard ordering), and OpenCV 5.1 cv2.projectPoints /
//! cv2.Rodrigues numbers in tests/fixtures/calib/projection.json written by
//! tool/refs/calib/projection.py. MATLAB-form rotation R = R_cv.' (row points
//! [Xc] = [X] * R + t). No MATLAB run.
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

fn output(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(done) => done.output.join("\n"),
        Err(error) => panic!("{code}\n  failed: {error}"),
    }
}

fn error_has(e: &mut Engine, code: &str, part: &str) {
    match e.execute(code) {
        Ok(_) => panic!("{code}\n  succeeded; expected an error"),
        Err(message) => assert!(
            message.contains(part),
            "{code}\n  error {message:?} does not contain {part:?}"
        ),
    }
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

/// Compare with `expected` given row by row (pages after each other).
fn check(e: &mut Engine, expr: &str, dims: &[usize], expected: &[f64], tol: f64) {
    let (d, v) = values(e, expr);
    assert_eq!(d, dims, "{expr}: size");
    let rows = dims[0];
    let cols = dims[1];
    let pages = v.len() / (rows * cols).max(1);
    for p in 0..pages {
        for i in 0..rows {
            for j in 0..cols {
                let a = v[p * rows * cols + i + j * rows];
                let b = expected[p * rows * cols + i * cols + j];
                let ok = if tol == 0. {
                    a == b || a.is_nan() && b.is_nan()
                } else {
                    (a - b).abs() <= tol * b.abs().max(1.)
                };
                assert!(ok, "{expr}: element ({i},{j},{p}) is {a:e}, expected {b:e}");
            }
        }
    }
}

fn truth(e: &mut Engine, expr: &str) -> bool {
    run(e, &format!("zz_t = double(logical({expr}));"));
    matrix(e, "zz_t").data() == [1.]
}

fn mat_literal(rows: &[Vec<f64>]) -> String {
    let body = rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|x| format!("{x:e}"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("[{body}]")
}

fn json_rows(v: &Json) -> Vec<Vec<f64>> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect()
        })
        .collect()
}

fn json_vec(v: &Json) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

#[test]
fn camera_intrinsics_construction_and_k() {
    let mut e = Engine::default();
    run(
        &mut e,
        "c = cameraIntrinsics([800 810], [320.5 240.25], [480 640], 'RadialDistortion', [-0.2 0.05 0.01], 'TangentialDistortion', [0.001 -0.002], 'Skew', 0.5);",
    );
    check(&mut e, "c.FocalLength", &[1, 2], &[800., 810.], 0.);
    check(&mut e, "c.PrincipalPoint", &[1, 2], &[320.5, 240.25], 0.);
    check(&mut e, "c.ImageSize", &[1, 2], &[480., 640.], 0.);
    check(
        &mut e,
        "c.RadialDistortion",
        &[1, 3],
        &[-0.2, 0.05, 0.01],
        0.,
    );
    check(
        &mut e,
        "c.TangentialDistortion",
        &[1, 2],
        &[0.001, -0.002],
        0.,
    );
    check(&mut e, "c.Skew", &[1, 1], &[0.5], 0.);
    let k = [800., 0.5, 320.5, 0., 810., 240.25, 0., 0., 1.];
    check(&mut e, "c.K", &[3, 3], &k, 0.);
    // IntrinsicMatrix is the transpose of K.
    check(
        &mut e,
        "c.IntrinsicMatrix",
        &[3, 3],
        &[800., 0., 0., 0.5, 810., 0., 320.5, 240.25, 1.],
        0.,
    );
    // Defaults.
    run(
        &mut e,
        "d = cameraIntrinsics([500 500], [100 80], [160 200]);",
    );
    check(&mut e, "d.RadialDistortion", &[1, 2], &[0., 0.], 0.);
    check(&mut e, "d.TangentialDistortion", &[1, 2], &[0., 0.], 0.);
    check(&mut e, "d.Skew", &[1, 1], &[0.], 0.);
    // The K form.
    run(&mut e, "f = cameraIntrinsics(c.K, [480 640]);");
    check(&mut e, "f.FocalLength", &[1, 2], &[800., 810.], 0.);
    check(&mut e, "f.Skew", &[1, 1], &[0.5], 0.);
    check(&mut e, "f.K", &[3, 3], &k, 0.);
    // Read-only properties and value equality.
    error_has(&mut e, "c.Skew = 1;", "read-only");
    error_has(&mut e, "c.Nope = 1;", "Unrecognized property");
    error_has(
        &mut e,
        "x = c.Nope;",
        "Unrecognized method, property, or field",
    );
    assert!(truth(
        &mut e,
        "isequal(d, cameraIntrinsics([500 500], [100 80], [160 200]))"
    ));
    assert!(!truth(&mut e, "isequal(d, c)"));
    assert!(!truth(&mut e, "isequal(d, 5)"));
    // Errors.
    error_has(
        &mut e,
        "cameraIntrinsics([800 800], [320 240])",
        "expected cameraIntrinsics",
    );
    error_has(
        &mut e,
        "cameraIntrinsics([800 -1], [320 240], [480 640])",
        "positive",
    );
    error_has(
        &mut e,
        "cameraIntrinsics([800 800], [320 240], [480 640], 'RadialDistortion', [1 2 3 4])",
        "2 or 3",
    );
    error_has(
        &mut e,
        "cameraIntrinsics([800 800], [320 240], [480 640], 'Bogus', 1)",
        "unknown name-value",
    );
    error_has(
        &mut e,
        "cameraIntrinsics([1 2 3; 4 5 6; 7 8 9], [480 640])",
        "[fx s cx; 0 fy cy; 0 0 1]",
    );
}

#[test]
fn camera_parameters_properties() {
    let mut e = Engine::default();
    run(&mut e, "p = cameraParameters;");
    check(
        &mut e,
        "p.K",
        &[3, 3],
        &[1., 0., 0., 0., 1., 0., 0., 0., 1.],
        0.,
    );
    check(&mut e, "p.NumPatterns", &[1, 1], &[0.], 0.);
    check(&mut e, "p.MeanReprojectionError", &[1, 1], &[f64::NAN], 0.);
    check(
        &mut e,
        "p.NumRadialDistortionCoefficients",
        &[1, 1],
        &[2.],
        0.,
    );
    assert!(truth(
        &mut e,
        "strcmp(p.WorldUnits, 'mm') && ~p.EstimateSkew && ~p.EstimateTangentialDistortion"
    ));
    assert!(truth(
        &mut e,
        "isempty(p.ImageSize) && isempty(p.RotationMatrices) && isempty(p.ReprojectedPoints)"
    ));
    // F3: without ImageSize the Intrinsics property is [] (public page).
    assert!(truth(
        &mut e,
        "isempty(p.Intrinsics) && isa(p.Intrinsics, 'double')"
    ));
    let text = output(&mut e, "disp(p)");
    assert!(
        text.contains("Intrinsics: []") && !text.contains("cameraIntrinsics]"),
        "{text}"
    );

    // IntrinsicMatrix is K.'.
    run(
        &mut e,
        "K = [700 0.25 330; 0 720 250; 0 0 1]; q = cameraParameters('IntrinsicMatrix', K.', 'ImageSize', [500 660]);",
    );
    check(
        &mut e,
        "q.K",
        &[3, 3],
        &[700., 0.25, 330., 0., 720., 250., 0., 0., 1.],
        0.,
    );
    check(
        &mut e,
        "q.IntrinsicMatrix",
        &[3, 3],
        &[700., 0., 0., 0.25, 720., 0., 330., 250., 1.],
        0.,
    );
    check(&mut e, "q.FocalLength", &[1, 2], &[700., 720.], 0.);
    check(&mut e, "q.PrincipalPoint", &[1, 2], &[330., 250.], 0.);
    check(&mut e, "q.Skew", &[1, 1], &[0.25], 0.);
    assert!(truth(
        &mut e,
        "isequal(q, cameraParameters('K', K, 'ImageSize', [500 660]))"
    ));
    // Intrinsics is a cameraIntrinsics with the same values.
    run(&mut e, "ci = q.Intrinsics;");
    assert!(truth(&mut e, "isa(ci, 'cameraIntrinsics')"));
    assert!(truth(
        &mut e,
        "isequal(ci, cameraIntrinsics([700 720], [330 250], [500 660], 'Skew', 0.25))"
    ));
    check(
        &mut e,
        "q.Intrinsics.K",
        &[3, 3],
        &[700., 0.25, 330., 0., 720., 250., 0., 0., 1.],
        0.,
    );

    // Extrinsics: RotationMatrices are rotationVectorToMatrix of each row.
    run(
        &mut e,
        "r = cameraParameters('K', K, 'RadialDistortion', [-0.1 0.02 0.001], 'RotationVectors', [0 0 pi/2; 0.1 -0.2 0.3], 'TranslationVectors', [1 2 300; -5 4 250], 'WorldPoints', generateCheckerboardPoints([3 4], 20), 'WorldUnits', 'cm', 'EstimateSkew', true, 'EstimateTangentialDistortion', true);",
    );
    check(&mut e, "r.NumPatterns", &[1, 1], &[2.], 0.);
    check(
        &mut e,
        "r.NumRadialDistortionCoefficients",
        &[1, 1],
        &[3.],
        0.,
    );
    check(
        &mut e,
        "size(r.RotationMatrices)",
        &[1, 3],
        &[3., 3., 2.],
        0.,
    );
    check(
        &mut e,
        "r.RotationMatrices(:, :, 1)",
        &[3, 3],
        &[0., 1., 0., -1., 0., 0., 0., 0., 1.],
        1e-15,
    );
    assert!(truth(
        &mut e,
        "isequal(r.RotationMatrices(:, :, 2), rotationVectorToMatrix([0.1 -0.2 0.3]))"
    ));
    assert!(truth(
        &mut e,
        "strcmp(r.WorldUnits, 'cm') && r.EstimateSkew && r.EstimateTangentialDistortion"
    ));
    check(
        &mut e,
        "r.RotationVectors(2, :)",
        &[1, 3],
        &[0.1, -0.2, 0.3],
        0.,
    );
    check(
        &mut e,
        "r.TranslationVectors(:, 3)'",
        &[1, 2],
        &[300., 250.],
        0.,
    );
    // ReprojectedPoints: worldToImage with distortion, pattern by pattern.
    check(
        &mut e,
        "size(r.ReprojectedPoints)",
        &[1, 3],
        &[6., 2., 2.],
        0.,
    );
    run(
        &mut e,
        "w = r.WorldPoints; d2 = worldToImage(r, r.RotationMatrices(:, :, 2), r.TranslationVectors(2, :), w, 'ApplyDistortion', true);",
    );
    assert!(truth(&mut e, "isequal(r.ReprojectedPoints(:, :, 2), d2)"));
    // MeanReprojectionError: mean of the row lengths over all pages.
    run(
        &mut e,
        "err = zeros(6, 2, 2); err(1, :, 1) = [3 4]; err(2, :, 2) = [6 8]; s = cameraParameters('K', K, 'WorldPoints', w, 'RotationVectors', [0 0 0; 0 0 0], 'TranslationVectors', [0 0 1; 0 0 2], 'ReprojectionErrors', err);",
    );
    check(
        &mut e,
        "s.MeanReprojectionError",
        &[1, 1],
        &[15. / 12.],
        1e-15,
    );
    assert!(truth(&mut e, "isequal(s.ReprojectionErrors, err)"));
    // Read-only and validation.
    error_has(&mut e, "r.WorldUnits = 'mm';", "read-only");
    error_has(
        &mut e,
        "cameraParameters('RotationVectors', [0 0 0], 'TranslationVectors', [0 0 1; 0 0 2])",
        "same number of rows",
    );
    error_has(
        &mut e,
        "cameraParameters('RadialDistortion', [0 0 0], 'NumRadialDistortionCoefficients', 2)",
        "NumRadialDistortionCoefficients",
    );
    error_has(
        &mut e,
        "cameraParameters('K', eye(3), 'IntrinsicMatrix', eye(3))",
        "not both",
    );
    error_has(&mut e, "cameraParameters('K')", "pairs");
    // F1: IntrinsicMatrix errors name IntrinsicMatrix and its own form.
    error_has(
        &mut e,
        "cameraParameters('IntrinsicMatrix', [700 0 330; 0 720 250; 0 0 1])",
        "IntrinsicMatrix must have the form [fx 0 0; s fy 0; cx cy 1]",
    );
    error_has(
        &mut e,
        "cameraParameters('IntrinsicMatrix', eye(2))",
        "IntrinsicMatrix",
    );
    run(
        &mut e,
        "n = cameraParameters('NumRadialDistortionCoefficients', 3);",
    );
    check(&mut e, "n.RadialDistortion", &[1, 3], &[0., 0., 0.], 0.);
}

#[test]
fn class_queries_and_display() {
    let mut e = Engine::default();
    run(
        &mut e,
        "c = cameraIntrinsics([800 800], [320 240], [480 640]); p = cameraParameters('ImageSize', [480 640]);",
    );
    assert!(truth(
        &mut e,
        "strcmp(class(c), 'cameraIntrinsics') && strcmp(class(p), 'cameraParameters')"
    ));
    assert!(truth(
        &mut e,
        "isa(p, 'cameraParameters') && ~isa(p, 'cameraIntrinsics') && ~isa(c, 'handle')"
    ));
    assert!(truth(
        &mut e,
        "isobject(c) && ~isstruct(c) && ~isnumeric(p)"
    ));
    assert!(truth(&mut e, "isequal(size(c), [1 1])"));
    assert!(truth(
        &mut e,
        "isequal(properties(c), {'FocalLength'; 'PrincipalPoint'; 'ImageSize'; 'RadialDistortion'; 'TangentialDistortion'; 'Skew'; 'K'})"
    ));
    assert!(truth(
        &mut e,
        "numel(properties(p)) == 20 && isprop(p, 'ReprojectedPoints') && ~isprop(p, 'Bogus')"
    ));
    assert!(truth(
        &mut e,
        "isequal(methods(c), {'worldToImage'}) && ismethod(p, 'worldToImage')"
    ));
    error_has(&mut e, "c == c", "Undefined function 'eq'");
    let text = output(&mut e, "c");
    let body = "  cameraIntrinsics with properties:\n\n             FocalLength: [800 800]\n          PrincipalPoint: [320 240]\n               ImageSize: [480 640]\n        RadialDistortion: [0 0]\n    TangentialDistortion: [0 0]\n                    Skew: 0\n                       K: [3x3 double]";
    assert!(text.contains("c = ") && text.contains(body), "{text}");
    let text = output(&mut e, "disp(p)");
    assert!(
        text.contains("  cameraParameters with properties:"),
        "{text}"
    );
    assert!(
        text.contains("                     Intrinsics: [1x1 cameraIntrinsics]\n"),
        "{text}"
    );
    assert!(
        text.contains("    NumRadialDistortionCoefficients: 2")
            || text.contains("NumRadialDistortionCoefficients: 2"),
        "{text}"
    );
    let text = output(&mut e, "z = {c, p}");
    assert!(
        text.contains("[1x1 cameraIntrinsics]") && text.contains("[1x1 cameraParameters]"),
        "{text}"
    );
    let text = output(&mut e, "properties(c)");
    assert!(
        text.contains("Properties for class cameraIntrinsics:"),
        "{text}"
    );
    // Copies are independent values.
    run(&mut e, "c2 = c;");
    assert!(truth(&mut e, "isequal(c2, c)"));
}

#[test]
fn checkerboard_points_count_order_and_origin() {
    let mut e = Engine::default();
    run(&mut e, "w = generateCheckerboardPoints([5 7], 25);");
    check(&mut e, "size(w)", &[1, 2], &[24., 2.], 0.);
    // Down the first column of corners (y varies fastest), then the next.
    check(
        &mut e,
        "w(1:6, :)",
        &[6, 2],
        &[0., 0., 0., 25., 0., 50., 0., 75., 25., 0., 25., 25.],
        0.,
    );
    check(&mut e, "w(end, :)", &[1, 2], &[125., 75.], 0.);
    check(
        &mut e,
        "generateCheckerboardPoints([2 2], 7)",
        &[1, 2],
        &[0., 0.],
        0.,
    );
    assert!(truth(&mut e, "isa(w, 'double')"));
    error_has(
        &mut e,
        "generateCheckerboardPoints([1 4], 25)",
        "at least 2",
    );
    error_has(&mut e, "generateCheckerboardPoints([4 4], -1)", "positive");
    error_has(&mut e, "generateCheckerboardPoints([4.5 4], 1)", "integers");
}

#[test]
fn rotation_vector_conversions() {
    let mut e = Engine::default();
    // Reference page examples.
    check(
        &mut e,
        "rotationVectorToMatrix(pi/2 * [0 0 1])",
        &[3, 3],
        &[0., 1., 0., -1., 0., 0., 0., 0., 1.],
        1e-15,
    );
    check(
        &mut e,
        "rotvec2mat3d(pi/2 * [0 0 1])",
        &[3, 3],
        &[0., -1., 0., 1., 0., 0., 0., 0., 1.],
        1e-15,
    );
    check(
        &mut e,
        "rotationMatrixToVector([0 1 0; -1 0 0; 0 0 1])",
        &[1, 3],
        &[0., 0., std::f64::consts::FRAC_PI_2],
        1e-15,
    );
    check(
        &mut e,
        "rotmat2vec3d([0 -1 0; 1 0 0; 0 0 1])",
        &[1, 3],
        &[0., 0., std::f64::consts::FRAC_PI_2],
        1e-15,
    );
    check(
        &mut e,
        "rotationVectorToMatrix([0 0 0])",
        &[3, 3],
        &[1., 0., 0., 0., 1., 0., 0., 0., 1.],
        0.,
    );
    check(&mut e, "rotmat2vec3d(eye(3))", &[1, 3], &[0., 0., 0.], 0.);
    // Angle pi about x.
    check(
        &mut e,
        "rotmat2vec3d(diag([1 -1 -1]))",
        &[1, 3],
        &[std::f64::consts::PI, 0., 0.],
        1e-15,
    );
    // Against cv2.Rodrigues: R_cv is the premultiply matrix.
    let json: Json = serde_json::from_str(include_str!("fixtures/calib/projection.json")).unwrap();
    for case in json["cases"].as_array().unwrap() {
        let rvec = json_vec(&case["rvec"]);
        let r_cv = json_rows(&case["R_cv"]);
        let flat: Vec<f64> = r_cv.iter().flatten().copied().collect();
        let v = format!("[{:e} {:e} {:e}]", rvec[0], rvec[1], rvec[2]);
        check(&mut e, &format!("rotvec2mat3d({v})"), &[3, 3], &flat, 1e-14);
        let t: Vec<f64> = (0..9).map(|k| r_cv[k % 3][k / 3]).collect();
        check(
            &mut e,
            &format!("rotationVectorToMatrix({v})"),
            &[3, 3],
            &t,
            1e-14,
        );
        let m = mat_literal(&r_cv);
        check(&mut e, &format!("rotmat2vec3d({m})"), &[1, 3], &rvec, 1e-12);
        check(
            &mut e,
            &format!("rotationMatrixToVector({m}.')"),
            &[1, 3],
            &rvec,
            1e-12,
        );
    }
    assert!(truth(
        &mut e,
        "isa(rotvec2mat3d(single([0 0 1])), 'single')"
    ));
    error_has(&mut e, "rotationVectorToMatrix([1 2])", "3-element");
    error_has(&mut e, "rotmat2vec3d(diag([1 1 -1]))", "rotation");
    // F4: scaled Newton gives the rotation of c * R for any c > 0; a
    // (to rounding) singular matrix is an error, never a wrong vector.
    for scale in ["1e20", "1e-20", "1e300", "1e-300"] {
        check(
            &mut e,
            &format!("rotmat2vec3d({scale} * rotvec2mat3d([0.3 0 0]))"),
            &[1, 3],
            &[0.3, 0., 0.],
            1e-14,
        );
        check(
            &mut e,
            &format!("rotationMatrixToVector({scale} * rotationVectorToMatrix([0.3 0 0]))"),
            &[1, 3],
            &[0.3, 0., 0.],
            1e-14,
        );
    }
    // Badly conditioned but nonsingular: the polar factor is still R.
    check(
        &mut e,
        "rotmat2vec3d(rotvec2mat3d([0.3 0 0]) * diag([1 1 1e-8]))",
        &[1, 3],
        &[0.3, 0., 0.],
        1e-12,
    );
    for code in [
        "rotmat2vec3d(rotvec2mat3d([0.3 0 0]) * diag([1 1 1e-25]))",
        "rotationMatrixToVector(rotationVectorToMatrix([0.3 0 0]) * diag([1 1 1e-25]))",
    ] {
        match e.execute(&format!("zz_r = {code};")) {
            Err(message) => assert!(message.contains("rotation"), "{code}: {message}"),
            Ok(_) => {
                let v = matrix(&e, "zz_r");
                let d = v.data();
                assert!(
                    (d[0] - 0.3).abs() < 1e-12 && d[1].abs() < 1e-12 && d[2].abs() < 1e-12,
                    "{code} gave {d:?}"
                );
            }
        }
    }
}

#[test]
fn world_to_image_matches_opencv_project_points() {
    let json: Json = serde_json::from_str(include_str!("fixtures/calib/projection.json")).unwrap();
    let f = json_vec(&json["focal"]);
    let c = json_vec(&json["principal"]);
    let k = json_vec(&json["radial"]);
    let p = json_vec(&json["tangential"]);
    let mut e = Engine::default();
    run(
        &mut e,
        &format!(
            "cam = cameraIntrinsics([{:e} {:e}], [{:e} {:e}], [480 640], 'RadialDistortion', [{:e} {:e} {:e}], 'TangentialDistortion', [{:e} {:e}]);",
            f[0], f[1], c[0], c[1], k[0], k[1], k[2], p[0], p[1]
        ),
    );
    run(
        &mut e,
        "par = cameraParameters('K', cam.K, 'RadialDistortion', cam.RadialDistortion, 'TangentialDistortion', cam.TangentialDistortion, 'ImageSize', [480 640]);",
    );
    for case in json["cases"].as_array().unwrap() {
        let rvec = json_vec(&case["rvec"]);
        let t = json_vec(&case["t"]);
        let world = json_rows(&case["world"]);
        let m = world.len();
        let distorted: Vec<f64> = json_rows(&case["distorted"])
            .into_iter()
            .flatten()
            .collect();
        let plain: Vec<f64> = json_rows(&case["undistorted"])
            .into_iter()
            .flatten()
            .collect();
        run(
            &mut e,
            &format!(
                "R = rotationVectorToMatrix([{:e} {:e} {:e}]); t = [{:e} {:e} {:e}]; W = {};",
                rvec[0],
                rvec[1],
                rvec[2],
                t[0],
                t[1],
                t[2],
                mat_literal(&world)
            ),
        );
        let name = case["name"].as_str().unwrap();
        for obj in ["cam", "par"] {
            check(
                &mut e,
                &format!("worldToImage({obj}, R, t, W, 'ApplyDistortion', true)"),
                &[m, 2],
                &distorted,
                1e-12,
            );
            // Default: no distortion.
            check(
                &mut e,
                &format!("worldToImage({obj}, R, t, W)"),
                &[m, 2],
                &plain,
                1e-12,
            );
        }
        if case["planar"].as_bool().unwrap() {
            // M-by-2 planar points are z = 0.
            check(
                &mut e,
                "worldToImage(cam, R, t, W(:, 1:2), 'ApplyDistortion', true)",
                &[m, 2],
                &distorted,
                1e-12,
            );
        }
        // The method form.
        check(
            &mut e,
            "cam.worldToImage(R, t, W, 'ApplyDistortion', true)",
            &[m, 2],
            &distorted,
            1e-12,
        );
        let _ = name;
    }
}

#[test]
fn world_to_image_valid_index_and_errors() {
    let mut e = Engine::default();
    run(
        &mut e,
        "c = cameraIntrinsics([800 800], [320 240], [480 640]);",
    );
    check(
        &mut e,
        "worldToImage(c, eye(3), [0 0 10], [1 2 0])",
        &[1, 2],
        &[400., 400.],
        0.,
    );
    // Skew: u = fx*x + s*y + cx.
    run(
        &mut e,
        "s = cameraIntrinsics([800 800], [320 240], [480 640], 'Skew', 2);",
    );
    check(
        &mut e,
        "worldToImage(s, eye(3), [0 0 10], [1 2 0])",
        &[1, 2],
        &[400.4, 400.],
        1e-15,
    );
    run(
        &mut e,
        "[q, ok] = worldToImage(c, eye(3), [0 0 0], [0 0 10; 0 0 -10; 10 0 1; 0.1 0.1 1]);",
    );
    check(&mut e, "double(ok)", &[4, 1], &[1., 0., 0., 1.], 0.);
    // F2: the method form returns both outputs like the function form.
    run(
        &mut e,
        "W = [0 0 10; 0 0 -10; 10 0 1; 0.1 0.1 1]; [q2, ok2] = c.worldToImage(eye(3), [0 0 0], W); [q3, ok3] = worldToImage(c, eye(3), [0 0 0], W);",
    );
    assert!(truth(
        &mut e,
        "isequal(q2, q3) && isequal(ok2, ok3) && islogical(ok2)"
    ));
    check(&mut e, "double(ok2)", &[4, 1], &[1., 0., 0., 1.], 0.);
    run(
        &mut e,
        "pp = cameraParameters('K', c.K, 'ImageSize', [480 640]); [q4, ok4] = pp.worldToImage(eye(3), [0 0 0], W);",
    );
    assert!(truth(&mut e, "isequal(q4, q3) && isequal(ok4, ok3)"));
    assert!(truth(&mut e, "islogical(ok)"));
    assert!(truth(
        &mut e,
        "isa(worldToImage(c, eye(3), [0 0 10], single([1 2 3])), 'single')"
    ));
    error_has(
        &mut e,
        "worldToImage(c, eye(3), [0 0 10])",
        "not supported yet",
    );
    error_has(&mut e, "worldToImage(c, eye(3))", "expected worldToImage");
    error_has(
        &mut e,
        "worldToImage(c, eye(2), [0 0 10], [1 2 3])",
        "3-by-3",
    );
    error_has(
        &mut e,
        "worldToImage(c, eye(3), [0 0 10], [1 2 3 4])",
        "M-by-3",
    );
    error_has(
        &mut e,
        "worldToImage(c, eye(3), [0 0 10], [1 2 3], 'Bogus', 1)",
        "unknown name-value",
    );
    error_has(
        &mut e,
        "worldToImage(5, eye(3), [0 0 10], [1 2 3])",
        "Undefined function 'worldToImage'",
    );
}

const TWO_VIEW_CALIBRATION_POINTS: &str = "\
    w = generateCheckerboardPoints([4 5], 25); \
    intr = cameraIntrinsics([800 790], [322 238], [480 640]); \
    p1 = worldToImage(intr, rotvec2mat3d([0 .5 0]), [-40 -25 500], [w zeros(size(w,1),1)]); \
    p2 = worldToImage(intr, rotvec2mat3d([.3 0 0]), [-40 -25 500], [w zeros(size(w,1),1)]); \
    pts = cat(3, p1, p2);";

#[test]
fn feval_nd_forwards_only_camera_functions() {
    let mut e = Engine::with_limits(Default::default());
    // Pre-7d behaviour: non-camera feval on 3-D array raises the N-D error.
    error_has(
        &mut e,
        "rgb = cat(3, ones(2), zeros(2), zeros(2)); feval(@rgb2gray, rgb);",
        "does not support N-D arrays yet",
    );
    // Camera N-D functions still work via feval.
    run(&mut e, TWO_VIEW_CALIBRATION_POINTS);
    run(
        &mut e,
        "cam = feval(@estimateCameraParameters, pts, w, 'ImageSize', [480 640]);",
    );
    assert!(truth(&mut e, "isa(cam, 'cameraParameters')"));
}

#[test]
fn cell_display_distinguishes_char_pua_from_camera_objects() {
    let mut e = Engine::with_limits(Default::default());
    let txt = output(
        &mut e,
        "a = inputParser; C = {a, repmat(char(57488), 1, 20)}",
    );
    assert!(txt.contains("[1x1 inputParser]"), "{txt}");
    let quoted = format!("'{}'", "\u{e090}".repeat(20));
    assert!(txt.contains(&quoted), "{txt}");
    assert!(!txt.contains("[1x1 cameraIntrinsics]"), "{txt}");
    let cam_txt = output(
        &mut e,
        "cam = cameraIntrinsics([800 800], [320 240], [480 640]); D = {cam}",
    );
    assert!(cam_txt.contains("[1x1 cameraIntrinsics]"), "{cam_txt}");
}

#[test]
fn four_column_cell_displays_full_camera_object_labels() {
    let mut e = Engine::with_limits(Default::default());
    run(&mut e, TWO_VIEW_CALIBRATION_POINTS);
    let txt = output(
        &mut e,
        "[params, ~, errors] = estimateCameraParameters(pts, w, 'ImageSize', [480 640]); \
         C = {intr, params, params.Intrinsics, errors}",
    );
    assert!(txt.contains("[1x1 cameraCalibrationErrors]"), "{txt}");
    assert!(
        !txt.chars()
            .any(|ch| ('\u{e000}'..='\u{f8ff}').contains(&ch)),
        "{txt}"
    );
}

#[test]
fn camera_object_cell_aligns_columns_with_a_shortened_char_row() {
    let mut e = Engine::default();
    let txt = output(
        &mut e,
        "cam = cameraIntrinsics([800 800], [320 240], [480 640]); \
         C = {cam, 1; 2, repmat('x', 1, 256)}",
    );
    let first = txt
        .lines()
        .find(|line| line.contains("[1x1 cameraIntrinsics]"))
        .unwrap_or_else(|| panic!("{txt}"));
    let shortened = format!("'{}…'", "x".repeat(45));
    let second = txt
        .lines()
        .find(|line| line.ends_with(&shortened))
        .unwrap_or_else(|| panic!("{txt}"));
    assert_eq!(first.rfind('['), second.find('\''), "{txt}");
}

#[test]
fn cell_display_preserves_string_text_beside_input_parser() {
    let mut e = Engine::with_limits(Default::default());
    for payload in [
        "\u{e000}".repeat(15),
        "\u{e000}".into(),
        format!("\u{e000}0000000{}", " ".repeat(7)),
    ] {
        let text = format!("\"'{payload}'\"");
        let txt = output(&mut e, &format!("p = inputParser; C = {{p, {text}}}"));
        assert!(txt.contains(&text), "{txt}");
        assert_eq!(txt.matches("[1x1 inputParser]").count(), 1, "{txt}");
        let txt = output(&mut e, "s.C = C; s");
        assert!(txt.contains(&text), "{txt}");
        assert_eq!(txt.matches("[1x1 inputParser]").count(), 1, "{txt}");
    }
}

#[test]
fn cell_display_preserves_char_text_when_all_bmp_pua_are_used() {
    let mut e = Engine::default();
    let txt = output(
        &mut e,
        "p = inputParser; C = {p, char(63743), char(57344:63743)}",
    );
    assert!(txt.contains("'\u{f8ff}'"), "{txt}");
    assert_eq!(txt.matches("[1x1 inputParser]").count(), 1, "{txt}");
}
