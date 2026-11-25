//! Camera calibration pieces 7d3a and 7d3b: estimateCameraParameters on
//! point sets, partial patterns, initial values and standard errors (OpenCV
//! cv2.calibrateCameraExtended standard deviations).
//!
//! Expected values: synthetic views of a 9x6-corner board (25 mm squares)
//! with a known camera, and OpenCV 5.1 cv2.calibrateCamera on the same
//! points (flags matching each case) in tests/fixtures/calib/estimate_*.json,
//! written by tool/refs/calib/estimate.py. MATLAB pixels = OpenCV pixels + 1.
//! The done-check: MeanReprojectionError within 0.05 px of OpenCV's mean
//! per-point Euclidean error. No MATLAB run.
use matrust::{Array, Engine, Matrix};
use serde_json::Value as Json;
use std::path::Path;

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

fn error_has(e: &mut Engine, code: &str, part: &str) {
    match e.execute(code) {
        Ok(_) => panic!("{code}\n  succeeded; expected an error"),
        Err(message) => assert!(
            message.contains(part),
            "{code}\n  error {message:?} does not contain {part:?}"
        ),
    }
}

fn values(e: &mut Engine, expr: &str) -> Vec<f64> {
    run(e, &format!("zz_v = double({expr}); zz_v = zz_v(:)';"));
    matrix(e, "zz_v").data().to_vec()
}

fn rows_literal(rows: &Json) -> String {
    let rows: Vec<String> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r.as_array()
                .unwrap()
                .iter()
                .map(|v| format!("{}", v.as_f64().unwrap()))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    format!("[{}]", rows.join("; "))
}

fn nums(v: &Json) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

/// Load a fixture into `w` (world points) and `ip` (M-by-2-by-P).
fn load(e: &mut Engine, json: &Json) {
    run(e, &format!("w = {};", rows_literal(&json["world_points"])));
    let views = json["views"].as_array().unwrap();
    run(
        e,
        &format!(
            "ip = zeros({}, 2, {});",
            views[0].as_array().unwrap().len(),
            views.len()
        ),
    );
    for (k, v) in views.iter().enumerate() {
        run(e, &format!("ip(:, :, {}) = {};", k + 1, rows_literal(v)));
    }
}

/// Detect all twelve rendered boards into `ip` and generate their world points `w`.
pub(super) fn load_rendered_boards(e: &mut Engine) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/calib/images");
    let files: Vec<_> = (1..=12)
        .map(|k| {
            let path = dir.join(format!("board_{k:02}.png"));
            format!("'{}'", path.to_string_lossy().replace('\'', "''"))
        })
        .collect();
    run(
        e,
        &format!(
            "[ip, board, detected] = detectCheckerboardPoints({{{}}}); w = generateCheckerboardPoints(board, 25);",
            files.join(", ")
        ),
    );
    assert_eq!(values(e, "detected"), vec![1.; 12]);
    assert_eq!(values(e, "board"), vec![7., 10.]);
    assert_eq!(values(e, "size(ip)"), vec![54., 2., 12.]);
}

fn fixture(name: &str) -> Json {
    let text = match name {
        "k2" => include_str!("fixtures/calib/estimate_k2.json"),
        "k3_tangential" => include_str!("fixtures/calib/estimate_k3_tangential.json"),
        "clean" => include_str!("fixtures/calib/estimate_clean.json"),
        "two_views_a" => include_str!("fixtures/calib/estimate_two_views_a.json"),
        "two_views_b" => include_str!("fixtures/calib/estimate_two_views_b.json"),
        "k2_partial" => include_str!("fixtures/calib/estimate_k2_partial.json"),
        _ => unreachable!(),
    };
    serde_json::from_str(text).unwrap()
}

/// The done-check case: mean error vs OpenCV, focal and principal vs truth.
fn calibrate_case(name: &str, options: &str) {
    let json = fixture(name);
    let mut e = Engine::default();
    load(&mut e, &json);
    run(
        &mut e,
        &format!(
            "[params, used, est] = estimateCameraParameters(ip, w, 'ImageSize', [480 640]{options});"
        ),
    );
    let mean = values(&mut e, "params.MeanReprojectionError")[0];
    let cv_mean = json["opencv"]["mean_error"].as_f64().unwrap();
    let focal = values(&mut e, "params.FocalLength");
    let principal = values(&mut e, "params.PrincipalPoint");
    let radial = values(&mut e, "params.RadialDistortion");
    let tangential = values(&mut e, "params.TangentialDistortion");
    println!(
        "{name}: mean {mean} (OpenCV {cv_mean}), focal {focal:?}, principal {principal:?}, radial {radial:?}, tangential {tangential:?}"
    );
    assert!(
        (mean - cv_mean).abs() <= 0.05,
        "{name}: mean reprojection error {mean} vs OpenCV {cv_mean}"
    );
    let truth = &json["truth"];
    for (got, want) in focal.iter().zip(nums(&truth["focal"])) {
        assert!(
            (got / want - 1.).abs() <= 0.005,
            "{name}: focal {got} vs {want}"
        );
    }
    for (got, want) in principal.iter().zip(nums(&truth["principal"])) {
        assert!(
            (got - want).abs() <= 2.,
            "{name}: principal {got} vs {want}"
        );
    }
    // Shapes and bookkeeping as the reference page describes.
    let p = json["views"].as_array().unwrap().len() as f64;
    assert_eq!(values(&mut e, "params.NumPatterns"), vec![p]);
    assert_eq!(values(&mut e, "size(params.RotationVectors)"), vec![p, 3.]);
    assert_eq!(
        values(&mut e, "size(params.TranslationVectors)"),
        vec![p, 3.]
    );
    assert_eq!(
        values(&mut e, "size(params.ReprojectionErrors)"),
        vec![54., 2., p]
    );
    assert_eq!(values(&mut e, "isequal(params.WorldPoints, w)"), vec![1.]);
    assert_eq!(values(&mut e, "params.ImageSize"), vec![480., 640.]);
    assert_eq!(
        values(&mut e, "all(used) && numel(used) == size(ip, 3)"),
        vec![1.]
    );
    assert_eq!(
        values(&mut e, "isa(est, 'cameraCalibrationErrors')"),
        vec![1.]
    );
    check_standard_errors(&mut e, name, &json);
    // ReprojectionErrors = ReprojectedPoints - imagePoints.
    run(
        &mut e,
        "zz_d = params.ReprojectedPoints - ip - params.ReprojectionErrors;",
    );
    let d = values(&mut e, "max(abs(zz_d(:)))");
    assert!(d[0] < 1e-9, "{name}: errors vs reprojected points {d:?}");
    // Translations near the truth (mm); depth sign right.
    let tv = values(&mut e, "params.TranslationVectors");
    let truth_t = truth["translation_vectors"].as_array().unwrap();
    for (k, t) in truth_t.iter().enumerate() {
        let t = nums(t);
        for j in 0..3 {
            let got = tv[k + j * truth_t.len()];
            assert!(
                (got - t[j]).abs() <= 0.02 * t[2],
                "{name}: view {k} t {got} vs {}",
                t[j]
            );
        }
    }
}

/// Largest relative difference |got / want - 1| over the pairs.
fn worst_ratio(name: &str, what: &str, got: &[f64], want: &[f64]) -> f64 {
    assert_eq!(got.len(), want.len(), "{name}: {what} lengths");
    let worst = got
        .iter()
        .zip(want)
        .map(|(g, w)| (g / w - 1.).abs())
        .fold(0f64, f64::max);
    println!("{name}: {what} standard errors worst ratio {worst:.5}");
    worst
}

/// Standard errors (third output, `est`) within 2 % of OpenCV's
/// calibrateCameraExtended standard deviations; parameters not estimated
/// have error 0.
fn check_standard_errors(e: &mut Engine, name: &str, json: &Json) {
    let cv = &json["opencv"];
    let tangential = json["tangential"].as_bool().unwrap();
    let mut worst = 0f64;
    for (prop, key) in [
        ("FocalLengthError", "std_focal"),
        ("PrincipalPointError", "std_principal"),
        ("RadialDistortionError", "std_radial"),
    ] {
        let got = values(e, &format!("est.IntrinsicsErrors.{prop}"));
        worst = worst.max(worst_ratio(name, prop, &got, &nums(&cv[key])));
    }
    let got = values(e, "est.IntrinsicsErrors.TangentialDistortionError");
    if tangential {
        worst = worst.max(worst_ratio(
            name,
            "TangentialDistortionError",
            &got,
            &nums(&cv["std_tangential"]),
        ));
    } else {
        assert_eq!(got, vec![0., 0.], "{name}: tangential not estimated");
    }
    assert_eq!(values(e, "est.IntrinsicsErrors.SkewError"), vec![0.]);
    for (prop, key) in [
        ("RotationVectorsError", "std_rotation_vectors"),
        ("TranslationVectorsError", "std_translation_vectors"),
    ] {
        // P-by-3 column-major against OpenCV's per-view rows.
        let rows = cv[key].as_array().unwrap();
        let want: Vec<f64> = (0..3)
            .flat_map(|j| rows.iter().map(move |r| nums(r)[j]))
            .collect();
        let got = values(e, &format!("est.ExtrinsicsErrors.{prop}"));
        worst = worst.max(worst_ratio(name, prop, &got, &want));
    }
    assert!(
        worst <= 0.02,
        "{name}: standard errors {worst} off OpenCV's"
    );
}

#[test]
fn camera_estimate_two_radial_matches_opencv() {
    calibrate_case("k2", "");
}

#[test]
fn camera_estimate_three_radial_tangential_matches_opencv() {
    calibrate_case(
        "k3_tangential",
        ", 'NumRadialDistortionCoefficients', 3, 'EstimateTangentialDistortion', true",
    );
}

#[test]
fn camera_estimate_rendered_boards_match_opencv() {
    // OpenCV 5.1.0 detects, calibrates and reprojects the same twelve PNGs.
    // Reproduce the mean with bash tool/ref-python.sh -I tool/refs/calib/rendered_mean.py.
    const OPENCV_MEAN_PX: f64 = 0.053_392_637_802_486_91;
    let mut e = Engine::default();
    load_rendered_boards(&mut e);
    run(
        &mut e,
        "[p, used] = estimateCameraParameters(ip, w, 'ImageSize', [480 640], \
         'NumRadialDistortionCoefficients', 2, 'EstimateTangentialDistortion', true);",
    );
    assert_eq!(values(&mut e, "used"), vec![1.; 12]);
    let mean = values(&mut e, "p.MeanReprojectionError")[0];
    println!("rendered boards: mean {mean} px (OpenCV {OPENCV_MEAN_PX} px)");
    assert!(
        (mean - OPENCV_MEAN_PX).abs() <= 0.05,
        "mean reprojection error {mean} px vs OpenCV {OPENCV_MEAN_PX} px"
    );
    assert!(mean < 0.08, "mean reprojection error {mean} px");
}

#[test]
fn camera_estimate_noise_free_reprojects_exactly() {
    let json = fixture("clean");
    let mut e = Engine::default();
    load(&mut e, &json);
    run(
        &mut e,
        "params = estimateCameraParameters(ip, w, 'WorldUnits', 'cm');",
    );
    let mean = values(&mut e, "params.MeanReprojectionError")[0];
    println!("clean: mean {mean}");
    assert!(mean < 1e-6, "noise-free mean reprojection error {mean}");
    let truth = &json["truth"];
    let focal = values(&mut e, "params.FocalLength");
    for (got, want) in focal.iter().zip(nums(&truth["focal"])) {
        assert!((got - want).abs() < 1e-5, "focal {got} vs {want}");
    }
    let radial = values(&mut e, "params.RadialDistortion");
    for (got, want) in radial.iter().zip(nums(&truth["radial"])) {
        assert!((got - want).abs() < 1e-8, "radial {got} vs {want}");
    }
    let rv = values(&mut e, "params.RotationVectors");
    let truth_r = truth["rotation_vectors"].as_array().unwrap();
    for (k, r) in truth_r.iter().enumerate() {
        for (j, want) in nums(r).iter().enumerate() {
            let got = rv[k + j * truth_r.len()];
            assert!((got - want).abs() < 1e-8, "view {k} rvec {got} vs {want}");
        }
    }
    assert_eq!(values(&mut e, "isempty(params.ImageSize)"), vec![1.]);
    assert_eq!(values(&mut e, "strcmp(params.WorldUnits, 'cm')"), vec![1.]);
    assert_eq!(
        values(&mut e, "params.NumRadialDistortionCoefficients"),
        vec![2.]
    );
    assert_eq!(values(&mut e, "params.EstimateSkew"), vec![0.]);
    assert_eq!(values(&mut e, "params.TangentialDistortion"), vec![0., 0.]);
    // With skew estimated, the skew comes out zero and the fit stays exact.
    run(
        &mut e,
        "p2 = estimateCameraParameters(ip, w, 'EstimateSkew', true);",
    );
    assert!(values(&mut e, "abs(p2.Skew)")[0] < 1e-4);
    assert!(values(&mut e, "p2.MeanReprojectionError")[0] < 1e-6);
    assert_eq!(values(&mut e, "p2.EstimateSkew"), vec![1.]);
}

#[test]
fn camera_estimate_rejects_too_few_or_degenerate_views() {
    let mut e = Engine::default();
    run(
        &mut e,
        "w = generateCheckerboardPoints([7 10], 25); c = cameraIntrinsics([800 790], [322 238], [480 640]);",
    );
    // One view.
    run(
        &mut e,
        "ip1 = worldToImage(c, rotationVectorToMatrix([0.3 -0.2 0.1]), [-100 -60 500], w);",
    );
    error_has(
        &mut e,
        "estimateCameraParameters(ip1, w);",
        "at least 2 views",
    );
    // All poses parallel: the same rotation at different translations.
    run(
        &mut e,
        "R = rotationVectorToMatrix([0.3 -0.2 0.1]); ip = zeros(54, 2, 4); \
         T = [-100 -60 500; -150 -40 600; -60 -80 450; -120 -70 700]; \
         for k = 1:4, ip(:, :, k) = worldToImage(c, R, T(k, :), w); end",
    );
    error_has(&mut e, "estimateCameraParameters(ip, w);", "degenerate");
    // Argument checks.
    error_has(
        &mut e,
        "estimateCameraParameters(ip, w(1:10, :));",
        "one row per world point",
    );
    error_has(
        &mut e,
        "estimateCameraParameters(ip, w, 'NumRadialDistortionCoefficients', 4);",
        "2 or 3",
    );
    error_has(
        &mut e,
        "estimateCameraParameters(ip, w, 'Bogus', 1);",
        "unknown name-value",
    );
    error_has(
        &mut e,
        "estimateCameraParameters(ip, [w zeros(54, 1)]);",
        "not supported yet",
    );
}

/// Tilted views made in the engine reproduce the camera exactly.
#[test]
fn camera_estimate_round_trip_with_world_to_image() {
    let mut e = Engine::default();
    run(
        &mut e,
        "w = generateCheckerboardPoints([7 10], 25); \
         c = cameraParameters('K', [800 0 322; 0 790 238; 0 0 1], 'RadialDistortion', [-0.12 0.05]); \
         r = [0.3 0 0; 0 0.3 0; 0.2 -0.2 0.1; -0.25 0.1 -0.1]; ip = zeros(54, 2, 4); \
         for k = 1:4, ip(:, :, k) = worldToImage(c, rotationVectorToMatrix(r(k, :)), [-100 -60 520], w, 'ApplyDistortion', true); end; \
         [p, used] = estimateCameraParameters(ip, w);",
    );
    assert!(values(&mut e, "p.MeanReprojectionError")[0] < 1e-6);
    let f = values(&mut e, "p.FocalLength");
    assert!(
        (f[0] - 800.).abs() < 1e-4 && (f[1] - 790.).abs() < 1e-4,
        "{f:?}"
    );
    let rv = values(&mut e, "max(abs(p.RotationVectors(:) - r(:)))");
    assert!(rv[0] < 1e-7, "{rv:?}");
    assert_eq!(values(&mut e, "used'"), vec![1., 1., 1., 1.]);
}

/// Two views are the documented minimum. Zhang's closed form is exactly
/// determined there and distorted points mislead it: from that start alone
/// set a ends in a wrong minimum (focal lengths 1055 and 1369 for 800 and
/// 790) and set b has no closed-form K at all (a false "degenerate" error).
/// The second start (fixed principal point) recovers the camera, as OpenCV
/// does; with and without ImageSize (image centre, centroid of the points).
#[test]
fn camera_estimate_two_distorted_views_recover_the_camera() {
    for name in ["two_views_a", "two_views_b"] {
        let json = fixture(name);
        let truth = &json["truth"];
        let cv_mean = json["opencv"]["mean_error"].as_f64().unwrap();
        for options in [", 'ImageSize', [480 640]", ""] {
            let mut e = Engine::default();
            load(&mut e, &json);
            run(
                &mut e,
                &format!("[params, used] = estimateCameraParameters(ip, w{options});"),
            );
            let mean = values(&mut e, "params.MeanReprojectionError")[0];
            let focal = values(&mut e, "params.FocalLength");
            let principal = values(&mut e, "params.PrincipalPoint");
            println!(
                "{name}{options}: mean {mean} (OpenCV {cv_mean}), focal {focal:?}, principal {principal:?}"
            );
            for (got, want) in focal.iter().zip(nums(&truth["focal"])) {
                assert!(
                    (got / want - 1.).abs() <= 0.01,
                    "{name}{options}: focal {got} vs {want}"
                );
            }
            for (got, want) in principal.iter().zip(nums(&truth["principal"])) {
                assert!(
                    (got - want).abs() <= 2.,
                    "{name}{options}: principal {got} vs {want}"
                );
            }
            assert!(
                (mean - cv_mean).abs() <= 0.05,
                "{name}{options}: mean reprojection error {mean} vs OpenCV {cv_mean}"
            );
            let radial = values(&mut e, "params.RadialDistortion");
            for (got, want) in radial.iter().zip(nums(&truth["radial"])) {
                assert!(
                    (got - want).abs() < 1e-6,
                    "{name}{options}: radial {got} vs {want}"
                );
            }
            assert_eq!(values(&mut e, "used'"), vec![1., 1.]);
            assert_eq!(values(&mut e, "params.NumPatterns"), vec![2.]);
        }
    }
}

/// World points far from the world origin (3 m off, pattern 0.5 m away): the
/// origin lies behind the camera plane in three of the five views. The pose
/// sign follows the pattern, not the origin: every view has the pattern in
/// front of the camera and the true pose (its mirror image reprojects just as
/// exactly).
#[test]
fn camera_estimate_pattern_far_from_world_origin_is_in_front() {
    let mut e = Engine::default();
    run(
        &mut e,
        "w = generateCheckerboardPoints([7 10], 25) + 3000; \
         c = cameraIntrinsics([800 790], [322 238], [480 640]); \
         r = [0.3 0 0; 0 0.3 0; 0 -0.3 0; -0.3 0 0; 0.2 -0.2 0.1]; \
         T = zeros(5, 3); ip = zeros(54, 2, 5); \
         for k = 1:5, R = rotationVectorToMatrix(r(k, :)); T(k, :) = [-100 -60 500] - [3000 3000 0] * R; \
         ip(:, :, k) = worldToImage(c, R, T(k, :), w); end; \
         p = estimateCameraParameters(ip, w); \
         z = zeros(1, 5); \
         for k = 1:5, Rk = rotationVectorToMatrix(p.RotationVectors(k, :)); \
         z(k) = [mean(w) 0] * Rk(:, 3) + p.TranslationVectors(k, 3); end",
    );
    // The case is the one meant: the world origin is behind the camera plane.
    assert_eq!(values(&mut e, "sum(T(:, 3) < 0)"), vec![3.]);
    let z = values(&mut e, "z");
    println!("pattern centroid depths {z:?}");
    assert!(z.iter().all(|d| (400. ..600.).contains(d)), "{z:?}");
    let t = values(&mut e, "max(abs(p.TranslationVectors(:) - T(:)))");
    assert!(t[0] < 1e-6, "translation error {t:?}");
    let rv = values(&mut e, "max(abs(p.RotationVectors(:) - r(:)))");
    assert!(rv[0] < 1e-8, "rotation vector error {rv:?}");
    let f = values(&mut e, "p.FocalLength");
    assert!(
        (f[0] - 800.).abs() < 1e-5 && (f[1] - 790.).abs() < 1e-5,
        "{f:?}"
    );
    assert!(values(&mut e, "p.MeanReprojectionError")[0] < 1e-6);
}

fn load_review_views(e: &mut Engine) {
    run(
        e,
        "w = generateCheckerboardPoints([4 5], 25); \
         c = cameraIntrinsics([800 790], [322 238], [480 640]); \
         r = [0 0.5 0; 0.3 0 0; -0.3 0 0]; ip = zeros(12, 2, 3); \
         for k = 1:3, ip(:, :, k) = worldToImage(c, rotationVectorToMatrix(r(k, :)), [-40 -25 500], w); end;",
    );
}

#[test]
fn camera_estimate_unobserved_world_point_preserves_intrinsics_and_poses() {
    let mut e = Engine::default();
    load_review_views(&mut e);
    run(
        &mut e,
        "p = estimateCameraParameters(ip, w); \
         ip2 = cat(1, ip, NaN(1, 2, 3)); \
         [q, used] = estimateCameraParameters(ip2, [w; 30000 0]);",
    );
    assert_eq!(values(&mut e, "used"), vec![1.; 3]);
    for prop in [
        "K",
        "RadialDistortion",
        "TangentialDistortion",
        "RotationVectors",
        "TranslationVectors",
    ] {
        let delta = values(&mut e, &format!("p.{prop} - q.{prop}"));
        assert!(delta.iter().all(|d| d.abs() <= 1e-9), "{prop}: {delta:?}");
    }
}

#[test]
fn camera_estimate_rejects_collinear_and_ill_conditioned_views() {
    let mut e = Engine::default();
    load_review_views(&mut e);
    for page in [
        "[(1:12)' (1:12)']",
        "[(1:12)' (1:12)' + 1e-6 * [1 -1 0 1 -1 0 1 -1 0 1 -1 0]']",
        "[(1:12)' (1:12)' + 1e-4 * [1 -1 0 1 -1 0 1 -1 0 1 -1 0]']",
        "[(1:12)' (1:12)' + 1e-3 * [1 -1 0 1 -1 0 1 -1 0 1 -1 0]']",
        "[ip(:, 1, 1) 1e-9 * ip(:, 2, 1)]",
        "[ip(:, 1, 1) 1e-8 * ip(:, 2, 1)]",
        "[ip(:, 1, 1) 1e-6 * ip(:, 2, 1)]",
        "[ip(:, 1, 1) 1e-3 * ip(:, 2, 1)]",
    ] {
        run(
            &mut e,
            &format!("ip(:, :, 4) = {page}; [p, used] = estimateCameraParameters(ip, w);"),
        );
        assert_eq!(values(&mut e, "used"), vec![1., 1., 1., 0.], "{page}");
        let focal = values(&mut e, "p.FocalLength");
        assert_eq!(focal.len(), 2);
        for (got, want) in focal.iter().zip([800., 790.]) {
            assert!((got - want).abs() <= 1e-6, "{page}: focal {focal:?}");
        }
    }
}

/// imagePoints is always 3-D: calls through a handle and through feval pass
/// the N-D guard as the direct call does.
#[test]
fn camera_estimate_through_handle_and_feval() {
    let mut e = Engine::default();
    run(
        &mut e,
        "w = generateCheckerboardPoints([7 10], 25); c = cameraIntrinsics([800 790], [322 238], [480 640]); \
         r = [0.3 0 0; 0 0.3 0; 0.2 -0.2 0.1]; ip = zeros(54, 2, 3); \
         for k = 1:3, ip(:, :, k) = worldToImage(c, rotationVectorToMatrix(r(k, :)), [-100 -60 500], w); end; \
         f = @estimateCameraParameters; p1 = f(ip, w); \
         p2 = feval('estimateCameraParameters', ip, w, 'ImageSize', [480 640]); \
         p3 = feval(f, ip, w); \
         p4 = cellfun(@estimateCameraParameters, {ip}, {w}, 'UniformOutput', false);",
    );
    for p in ["p1", "p2", "p3", "p4{1}"] {
        let f = values(&mut e, &format!("{p}.FocalLength"));
        assert!(
            (f[0] - 800.).abs() < 1e-5 && (f[1] - 790.).abs() < 1e-5,
            "{p}: {f:?}"
        );
        assert_eq!(values(&mut e, &format!("{p}.NumPatterns")), vec![3.]);
    }
    assert_eq!(values(&mut e, "p2.ImageSize"), vec![480., 640.]);
    // Non-camera feval targets keep the pre-7d N-D error (7d whole-step review #14).
    error_has(
        &mut e,
        "y = feval(@inv, ones(2, 2, 2));",
        "feval does not support N-D arrays yet",
    );
    error_has(
        &mut e,
        "y = feval('ctrb', ones(2, 2, 2), ones(2, 1));",
        "feval does not support N-D arrays yet",
    );
}

/// Parallel poses with detection noise: the noise gives the closed-form
/// system full rank, so the rank test alone lets them through (a fit with
/// focal lengths 533 and 485 for 800 and 790 from the closed-form start).
/// The fitted pattern planes agree within a degree: degenerate.
#[test]
fn camera_estimate_rejects_noisy_parallel_views() {
    let mut e = Engine::default();
    run(
        &mut e,
        "w = generateCheckerboardPoints([7 10], 25); c = cameraIntrinsics([800 790], [322 238], [480 640]); \
         R = rotationVectorToMatrix([0.3 -0.2 0.1]); \
         T = [-100 -60 500; -150 -40 600; -60 -80 450; -120 -70 700; -90 -50 550; -130 -30 650]; \
         ip = zeros(54, 2, 6); for k = 1:6, ip(:, :, k) = worldToImage(c, R, T(k, :), w); end",
    );
    for a in ["78.233", "12.9898"] {
        run(
            &mut e,
            &format!("noisy = ip + 0.1 * reshape(sin((1:numel(ip)) * {a}), size(ip));"),
        );
        error_has(&mut e, "estimateCameraParameters(noisy, w);", "degenerate");
        error_has(
            &mut e,
            "estimateCameraParameters(noisy, w, 'ImageSize', [480 640]);",
            "degenerate",
        );
    }
    // Planes four degrees apart are not parallel: two such views calibrate.
    run(
        &mut e,
        "ip2 = zeros(54, 2, 2); ip2(:, :, 1) = worldToImage(c, rotationVectorToMatrix([0.3 0 0]), [-100 -60 500], w); \
         ip2(:, :, 2) = worldToImage(c, rotationVectorToMatrix([0.3 0.07 0]), [-120 -50 520], w); \
         p = estimateCameraParameters(ip2, w, 'ImageSize', [480 640]);",
    );
    let f = values(&mut e, "p.FocalLength");
    assert!(
        (f[0] / 800. - 1.).abs() < 0.01 && (f[1] / 790. - 1.).abs() < 0.01,
        "{f:?}"
    );
}

/// Partially detected patterns: NaN rows are left out; the view with only
/// 3 points left is not used (imagesUsed false) and the results cover the
/// 14 used views. Mean error vs OpenCV on the same finite points.
#[test]
fn camera_estimate_partial_patterns_with_nan_rows() {
    let json = fixture("k2_partial");
    let mut e = Engine::default();
    load(&mut e, &json);
    for pair in json["missing"].as_array().unwrap() {
        let p = nums(pair);
        run(
            &mut e,
            &format!("ip({}, :, {}) = NaN;", p[1] + 1., p[0] + 1.),
        );
    }
    run(
        &mut e,
        "lastwarn(''); [params, used, est] = estimateCameraParameters(ip, w, 'ImageSize', [480 640]); \
         zz_w = lastwarn;",
    );
    let dropped = json["dropped_view"].as_f64().unwrap() as usize;
    // The dropped view is named in a warning (1-based).
    let warned = match e.array("zz_w") {
        Some(Array::Character(c)) => String::from_utf16_lossy(c.units()),
        other => panic!("{other:?}"),
    };
    assert!(
        warned.contains(&format!("view(s) {} not used", dropped + 1)),
        "warning {warned:?}"
    );
    let used = values(&mut e, "used'");
    assert_eq!(used.len(), 15);
    for (k, u) in used.iter().enumerate() {
        assert_eq!(*u, if k == dropped { 0. } else { 1. }, "view {k}");
    }
    let mean = values(&mut e, "params.MeanReprojectionError")[0];
    let cv_mean = json["opencv"]["mean_error"].as_f64().unwrap();
    println!("k2_partial: mean {mean} (OpenCV {cv_mean})");
    assert!((mean - cv_mean).abs() <= 0.05, "mean {mean} vs {cv_mean}");
    assert_eq!(values(&mut e, "params.NumPatterns"), vec![14.]);
    assert_eq!(
        values(&mut e, "size(params.RotationVectors)"),
        vec![14., 3.]
    );
    assert_eq!(
        values(&mut e, "size(params.ReprojectionErrors)"),
        vec![54., 2., 14.]
    );
    // Missing points have NaN errors (x and y): 5 in each of 5 views.
    run(&mut e, "zz_r = params.ReprojectionErrors;");
    assert_eq!(values(&mut e, "sum(isnan(zz_r(:)))"), vec![50.]);
    assert_eq!(
        values(&mut e, "size(est.ExtrinsicsErrors.RotationVectorsError)"),
        vec![14., 3.]
    );
    let focal = values(&mut e, "params.FocalLength");
    for (got, want) in focal.iter().zip(nums(&json["opencv"]["focal"])) {
        assert!((got / want - 1.).abs() < 1e-4, "focal {got} vs {want}");
    }
    let got = values(&mut e, "est.IntrinsicsErrors.FocalLengthError");
    let worst = worst_ratio(
        "k2_partial",
        "focal",
        &got,
        &nums(&json["opencv"]["std_focal"]),
    );
    assert!(worst <= 0.02, "{worst}");
    // A view with an Inf is an error, not a missing point.
    error_has(
        &mut e,
        "ip(1, 1, 2) = Inf; estimateCameraParameters(ip, w);",
        "[NaN NaN]",
    );
    // Fewer than 2 usable views.
    error_has(
        &mut e,
        "ip2 = ip(:, :, [1 15]); ip2(5:end, :, 1) = NaN; estimateCameraParameters(ip2, w);",
        "fewer than 2 views",
    );
}

/// InitialIntrinsicMatrix (K' layout) and InitialK (K layout) at the truth,
/// and InitialRadialDistortion: the same solution as the closed-form start.
#[test]
fn camera_estimate_initial_values_reach_the_same_solution() {
    let json = fixture("k2");
    let mut e = Engine::default();
    load(&mut e, &json);
    run(
        &mut e,
        "p0 = estimateCameraParameters(ip, w, 'ImageSize', [480 640]); \
         K = [800 0 322; 0 790 238; 0 0 1]; \
         p1 = estimateCameraParameters(ip, w, 'ImageSize', [480 640], 'InitialIntrinsicMatrix', K'); \
         p2 = estimateCameraParameters(ip, w, 'ImageSize', [480 640], 'InitialK', K, 'InitialRadialDistortion', [-0.12 0.05]);",
    );
    for p in ["p1", "p2"] {
        let d = values(
            &mut e,
            &format!("max(abs({p}.ReprojectedPoints(:) - p0.ReprojectedPoints(:)))"),
        )[0];
        println!("{p}: reprojected points differ by {d} px");
        assert!(d < 1e-6, "{p}: {d}");
        let m = values(
            &mut e,
            &format!("abs({p}.MeanReprojectionError - p0.MeanReprojectionError)"),
        )[0];
        assert!(m < 1e-9, "{p}: {m}");
    }
    error_has(
        &mut e,
        "estimateCameraParameters(ip, w, 'InitialIntrinsicMatrix', K);",
        "transpose of K",
    );
    error_has(
        &mut e,
        "estimateCameraParameters(ip, w, 'InitialRadialDistortion', [0 0 0]);",
        "NumRadialDistortionCoefficients",
    );
}

/// displayErrors prints the standard error table; the error objects are
/// read-only and show their properties.
#[test]
fn camera_estimate_display_errors() {
    let json = fixture("k3_tangential");
    let mut e = Engine::default();
    load(&mut e, &json);
    run(
        &mut e,
        "[params, ~, est] = estimateCameraParameters(ip, w, 'ImageSize', [480 640], \
         'NumRadialDistortionCoefficients', 3, 'EstimateTangentialDistortion', true, 'WorldUnits', 'mm'); \
         s = evalc('displayErrors(est, params)'); \
         f = params.FocalLength; fe = est.IntrinsicsErrors.FocalLengthError;",
    );
    let text = match e.variables.get("s") {
        Some(_) => unreachable!(),
        None => match e.array("s") {
            Some(Array::Character(c)) => String::from_utf16_lossy(c.units()),
            other => panic!("{other:?}"),
        },
    };
    println!("{text}");
    let f = values(&mut e, "f");
    let fe = values(&mut e, "fe");
    let line = format!(
        "Focal length (pixels):   [{:>10.4} +/- {:<8.4}  {:>10.4} +/- {:<8.4}]",
        f[0], fe[0], f[1], fe[1]
    );
    assert!(text.contains(&line), "missing {line:?}");
    for part in [
        "Standard Errors of Estimated Camera Parameters",
        "Principal point (pixels):[",
        "Radial distortion:       [",
        "Tangential distortion:   [",
        "Rotation vectors:\n                         [",
        "Translation vectors (mm):\n",
    ] {
        assert!(text.contains(part), "missing {part:?}");
    }
    assert_eq!(text.matches(" +/- ").count(), 2 + 2 + 3 + 2 + 15 * 6);
    assert_eq!(
        values(
            &mut e,
            "isa(est.IntrinsicsErrors, 'intrinsicsEstimationErrors')"
        ),
        vec![1.]
    );
    assert_eq!(
        values(
            &mut e,
            "isa(est.ExtrinsicsErrors, 'extrinsicsEstimationErrors')"
        ),
        vec![1.]
    );
    error_has(&mut e, "est.IntrinsicsErrors = 1;", "read-only");
    run(
        &mut e,
        "s2 = evalc('est'); s3 = evalc('x = est.IntrinsicsErrors');",
    );
    for (v, part) in [
        ("s2", "IntrinsicsErrors: [1x1 intrinsicsEstimationErrors]"),
        ("s3", "FocalLengthError"),
    ] {
        let t = match e.array(v) {
            Some(Array::Character(c)) => String::from_utf16_lossy(c.units()),
            other => panic!("{other:?}"),
        };
        println!("{t}");
        assert!(t.contains(part), "{v}: missing {part:?} in {t:?}");
    }
    error_has(&mut e, "displayErrors(est);", "displayErrors");
    // Every statement form prints the table; only a use of the value fails.
    for form in [
        "est.displayErrors(params)",
        "feval('displayErrors', est, params)",
        "zz_f = @displayErrors; zz_f(est, params)",
        "displayErrors(est, params);",
    ] {
        run(
            &mut e,
            &format!("zz_t = evalc('{}');", form.replace('\'', "''")),
        );
        let t = match e.array("zz_t") {
            Some(Array::Character(c)) => String::from_utf16_lossy(c.units()),
            other => panic!("{other:?}"),
        };
        assert!(t.contains(&line), "{form}: missing {line:?} in {t:?}");
    }
    error_has(
        &mut e,
        "zz_x = displayErrors(est, params);",
        "does not return a value",
    );
    error_has(
        &mut e,
        "zz_x = est.displayErrors(params);",
        "does not return a value",
    );
    error_has(
        &mut e,
        "[zz_a, zz_b] = est.displayErrors(params);",
        "Too many output arguments",
    );
}
