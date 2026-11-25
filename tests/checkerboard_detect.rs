//! detectCheckerboardPoints (engine 7d2): checkerboard inner corners to
//! sub-pixel accuracy, in generateCheckerboardPoints order.
//!
//! Expected values: tests/fixtures/calib, made by tool/refs/calib/render.py
//! (NumPy + OpenCV, no MATLAB): twelve 640x480 renderings of a 7x10 board of
//! 25 mm squares through a known distorted camera (four of them with the board
//! pushed into an image corner, outer squares cut by the frame), with the exact corner
//! positions (truth.json, MATLAB 1-based pixels, generateCheckerboardPoints
//! order: origin = lower-right corner of the black top-left square, x along
//! the 10-square side, y fastest) and OpenCV's findChessboardCorners +
//! cornerSubPix corners (opencv.json, same order) as the bar to meet.
use matrust::{Array, Engine, Matrix};
use serde_json::Value as Json;
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/calib")
}

fn image_path(name: &str) -> String {
    fixtures()
        .join("images")
        .join(name)
        .to_string_lossy()
        .replace('\'', "''")
}

fn json(name: &str) -> Json {
    let text = std::fs::read_to_string(fixtures().join(name)).expect("fixture");
    serde_json::from_str(&text).expect("json")
}

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

/// Corner points of truth.json image `k` (0-based), 1-based pixels, (x, y).
fn truth_points(truth: &Json, k: usize) -> Vec<(f64, f64)> {
    truth["images"][k]["corners_1based"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap()))
        .collect()
}

/// Points of an M-by-2 MATLAB variable as (x, y).
fn points(e: &mut Engine, expr: &str) -> Vec<(f64, f64)> {
    let v = values(e, expr);
    let m = v.len() / 2;
    (0..m).map(|i| (v[i], v[m + i])).collect()
}

fn errors(found: &[(f64, f64)], truth: &[(f64, f64)]) -> Vec<f64> {
    assert_eq!(found.len(), truth.len());
    found
        .iter()
        .zip(truth)
        .map(|(a, b)| (a.0 - b.0).hypot(a.1 - b.1))
        .collect()
}

fn rms(errs: &[f64]) -> f64 {
    (errs.iter().map(|e| e * e).sum::<f64>() / errs.len() as f64).sqrt()
}

#[test]
fn checkerboard_detect_fixture_images_match_truth() {
    let truth = json("truth.json");
    let opencv = json("opencv.json");
    let mut e = Engine::default();
    let mut all = Vec::new();
    let mut opencv_all = Vec::new();
    for k in 0..12 {
        let file = truth["images"][k]["file"].as_str().unwrap();
        run(
            &mut e,
            &format!(
                "[p, b] = detectCheckerboardPoints(imread('{}'));",
                image_path(file)
            ),
        );
        assert_eq!(values(&mut e, "b"), vec![7., 10.], "{file}: boardSize");
        assert_eq!(values(&mut e, "size(p)"), vec![54., 2.], "{file}: size");
        let errs = errors(&points(&mut e, "p"), &truth_points(&truth, k));
        let cv: Vec<(f64, f64)> = opencv["images"][k]["corners_1based"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap()))
            .collect();
        let cv_errs = errors(&cv, &truth_points(&truth, k));
        eprintln!(
            "{file}: rms {:.4} max {:.4} (opencv rms {:.4} max {:.4})",
            rms(&errs),
            errs.iter().copied().fold(0., f64::max),
            rms(&cv_errs),
            cv_errs.iter().copied().fold(0., f64::max)
        );
        all.extend(errs);
        opencv_all.extend(cv_errs);
    }
    let (r, m, r_cv) = (
        rms(&all),
        all.iter().copied().fold(0., f64::max),
        rms(&opencv_all),
    );
    eprintln!("all: rms {r:.4} max {m:.4}; opencv rms {r_cv:.4}");
    assert!(r <= 0.10, "rms {r}");
    assert!(m <= 0.35, "max {m}");
    assert!(r <= r_cv + 0.03, "rms {r} vs opencv {r_cv}");
}

#[test]
fn checkerboard_detect_blank_and_noise_find_nothing() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[p, b] = detectCheckerboardPoints(zeros(480, 640, 'uint8'));",
    );
    assert!(values(&mut e, "p").is_empty());
    assert_eq!(values(&mut e, "size(p)"), vec![0., 0.]);
    assert_eq!(values(&mut e, "b"), vec![0., 0.]);
    run(
        &mut e,
        "[p, b] = detectCheckerboardPoints(0.5 * ones(100, 120));",
    );
    assert_eq!(values(&mut e, "b"), vec![0., 0.]);
    // Pseudo-random bytes from a fixed hash (no RNG state involved).
    let mut noise = Engine::default();
    run(
        &mut noise,
        "k = 1:480*640; N = uint8(reshape(mod(floor(43758.5453 * abs(sin(k * 12.9898))), 256), 480, 640));",
    );
    assert!(values(&mut noise, "std(double(N(:)))")[0] > 60.);
    run(&mut noise, "[p, b] = detectCheckerboardPoints(N);");
    assert!(values(&mut noise, "p").is_empty());
    assert_eq!(values(&mut noise, "b"), vec![0., 0.]);
}

#[test]
fn checkerboard_detect_rotated_image_keeps_origin_rule() {
    // rot90 turns the image 90 degrees counterclockwise: pixel (x, y) of the
    // W-wide original moves to (y, W + 1 - x). The board is the same, so the
    // same physical corners come back in the same order (black origin square,
    // x along the 10-square side, right-handed).
    let truth = json("truth.json");
    let mut e = Engine::default();
    for (k, turns) in [(0usize, 1usize), (5, 3), (2, 2)] {
        let file = truth["images"][k]["file"].as_str().unwrap();
        run(
            &mut e,
            &format!(
                "[p, b] = detectCheckerboardPoints(rot90(imread('{}'), {turns}));",
                image_path(file)
            ),
        );
        assert_eq!(values(&mut e, "b"), vec![7., 10.], "{file} turned {turns}");
        let (mut w, mut h) = (640., 480.);
        let expected: Vec<(f64, f64)> = truth_points(&truth, k)
            .into_iter()
            .map(|mut p| {
                let (mut ww, mut hh) = (w, h);
                for _ in 0..turns {
                    p = (p.1, ww + 1. - p.0);
                    std::mem::swap(&mut ww, &mut hh);
                }
                p
            })
            .collect();
        if turns % 2 == 1 {
            std::mem::swap(&mut w, &mut h);
        }
        let size_expr = format!("size(rot90(zeros(480, 640), {turns}))");
        assert_eq!(values(&mut e, &size_expr), vec![h, w]);
        let errs = errors(&points(&mut e, "p"), &expected);
        let worst = errs.iter().copied().fold(0., f64::max);
        eprintln!(
            "{file} turned {turns}: rms {:.4} max {worst:.4}",
            rms(&errs)
        );
        assert!(worst <= 0.35, "{file} turned {turns}: max {worst}");
    }
}

#[test]
fn checkerboard_detect_file_names_and_stacks_return_images_used() {
    let truth = json("truth.json");
    let dir = std::env::temp_dir().join(format!("matrust-le7d2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let blank = dir.join("blank.png").to_string_lossy().replace('\'', "''");
    let mut e = Engine::default();
    run(
        &mut e,
        &format!("imwrite(uint8(128 * ones(480, 640)), '{blank}');"),
    );
    let (a, b) = (image_path("board_03.png"), image_path("board_07.png"));
    run(
        &mut e,
        &format!("[p, s, used] = detectCheckerboardPoints({{'{a}', '{blank}', '{b}'}});"),
    );
    assert_eq!(values(&mut e, "used"), vec![1., 0., 1.]);
    assert_eq!(values(&mut e, "class(used) == 'logical'"), vec![1.; 7]);
    assert_eq!(values(&mut e, "size(used)"), vec![3., 1.]);
    assert_eq!(values(&mut e, "size(p)"), vec![54., 2., 2.]);
    assert_eq!(values(&mut e, "s"), vec![7., 10.]);
    let second = errors(&points(&mut e, "p(:, :, 2)"), &truth_points(&truth, 6));
    assert!(second.iter().all(|&d| d <= 0.35), "{second:?}");
    // String array of names, and an H-by-W-by-N stack.
    run(
        &mut e,
        &format!("[p2, s2, used2] = detectCheckerboardPoints([\"{a}\", \"{b}\"]);"),
    );
    assert_eq!(values(&mut e, "used2"), vec![1., 1.]);
    assert_eq!(values(&mut e, "isequal(p2, p)"), vec![1.]);
    run(
        &mut e,
        &format!(
            "S = cat(3, imread('{a}'), uint8(zeros(480, 640)), imread('{b}'), uint8(zeros(480, 640))); [p3, s3, used3] = detectCheckerboardPoints(S);"
        ),
    );
    assert_eq!(values(&mut e, "used3"), vec![1., 0., 1., 0.]);
    assert_eq!(values(&mut e, "isequal(p3, p)"), vec![1.]);
    // H-by-W-by-1-by-N grey and H-by-W-by-3-by-N truecolor stacks.
    run(
        &mut e,
        &format!(
            "G = cat(4, imread('{a}'), uint8(128 * ones(480, 640)), imread('{b}')); [p5, s5, used5] = detectCheckerboardPoints(G);"
        ),
    );
    assert_eq!(values(&mut e, "size(G)"), vec![480., 640., 1., 3.]);
    assert_eq!(values(&mut e, "used5"), vec![1., 0., 1.]);
    assert_eq!(values(&mut e, "s5"), vec![7., 10.]);
    assert_eq!(values(&mut e, "isequal(p5, p)"), vec![1.]);
    run(
        &mut e,
        "C = cat(4, cat(3, G(:, :, 1, 1), G(:, :, 1, 1), G(:, :, 1, 1)), cat(3, G(:, :, 1, 3), G(:, :, 1, 3), G(:, :, 1, 3))); [p6, s6, used6] = detectCheckerboardPoints(C);",
    );
    assert_eq!(values(&mut e, "size(C)"), vec![480., 640., 3., 2.]);
    assert_eq!(values(&mut e, "used6"), vec![1., 1.]);
    assert_eq!(values(&mut e, "max(abs(p6(:) - p(:))) < 1e-9"), vec![1.]);
    error_has(
        &mut e,
        "detectCheckerboardPoints(zeros(40, 40, 2, 2))",
        "H-by-W-by-1-by-N",
    );
    // No board anywhere.
    run(
        &mut e,
        &format!("[p4, s4, used4] = detectCheckerboardPoints({{'{blank}'}});"),
    );
    assert_eq!(values(&mut e, "size(p4)"), vec![0., 0.]);
    assert_eq!(values(&mut e, "s4"), vec![0., 0.]);
    assert_eq!(values(&mut e, "used4"), vec![0.]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn checkerboard_detect_options_and_errors() {
    let mut e = Engine::default();
    let a = image_path("board_01.png");
    run(
        &mut e,
        &format!("I = imread('{a}'); [p, b] = detectCheckerboardPoints(I);"),
    );
    run(
        &mut e,
        "[q, c] = detectCheckerboardPoints(I, 'PartialDetections', false, 'HighDistortion', true);",
    );
    assert_eq!(values(&mut e, "isequal(p, q) && isequal(b, c)"), vec![1.]);
    // Truecolor and double inputs give the same corners.
    run(
        &mut e,
        "[q2, c2] = detectCheckerboardPoints(cat(3, I, I, I));",
    );
    assert_eq!(values(&mut e, "max(abs(q2(:) - p(:))) < 1e-9"), vec![1.]);
    run(&mut e, "[q3, c3] = detectCheckerboardPoints(im2double(I));");
    assert_eq!(values(&mut e, "max(abs(q3(:) - p(:))) < 1e-9"), vec![1.]);
    // A clean synthetic board: corners exactly between pixels.
    run(
        &mut e,
        "J = ones(180, 240); J(21:160, 21:220) = kron(mod(repmat((0:6)', 1, 10) + repmat(0:9, 7, 1), 2), ones(20)); [r, s] = detectCheckerboardPoints(J);",
    );
    assert_eq!(values(&mut e, "s"), vec![7., 10.]);
    let first = values(&mut e, "r(1:2, :)");
    for (got, want) in first.iter().zip([40.5, 40.5, 40.5, 60.5]) {
        assert!((got - want).abs() < 1e-6, "{first:?}");
    }
    error_has(
        &mut e,
        "detectCheckerboardPoints(I, 'MinCornerMetric', 0.2)",
        "not supported yet",
    );
    error_has(&mut e, "detectCheckerboardPoints(I, I)", "stereo");
    error_has(
        &mut e,
        "detectCheckerboardPoints(I, 'Bogus', 1)",
        "unknown option",
    );
    error_has(
        &mut e,
        "detectCheckerboardPoints(I, 'HighDistortion', 3)",
        "true or false",
    );
    error_has(
        &mut e,
        "[a1, a2, a3] = detectCheckerboardPoints(I)",
        "too many output",
    );
    error_has(&mut e, "detectCheckerboardPoints()", "not enough input");
}

#[test]
fn checkerboard_detect_function_handles_take_images_and_stacks() {
    // Handle calls pass the N-D guard like direct calls (truecolor is 3-D).
    let mut e = Engine::default();
    let a = image_path("board_01.png");
    run(
        &mut e,
        &format!("I = imread('{a}'); p = detectCheckerboardPoints(I); rgb = cat(3, I, I, I);"),
    );
    assert_eq!(values(&mut e, "size(p)"), vec![54., 2.]);
    run(&mut e, "f = @detectCheckerboardPoints; q = f(rgb);");
    assert_eq!(values(&mut e, "max(abs(q(:) - p(:))) < 1e-9"), vec![1.]);
    run(
        &mut e,
        "c = cellfun(@detectCheckerboardPoints, {rgb, I}, 'UniformOutput', false);",
    );
    assert_eq!(values(&mut e, "isequal(c{1}, q)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(c{2}, p)"), vec![1.]);
    run(&mut e, "q4 = f(cat(4, I, I));");
    assert_eq!(values(&mut e, "size(q4)"), vec![54., 2., 2.]);
    assert_eq!(values(&mut e, "isequal(q4(:, :, 2), p)"), vec![1.]);
}

#[test]
fn checkerboard_detect_corners_close_to_the_border_keep_the_board() {
    // board_01 cropped so that its outermost corner is 7 to 10 px from the
    // image edge: less room than the refinement window (8 px + 1) wants. The
    // window shrinks for those corners; the board stays whole.
    let truth = json("truth.json");
    let t = truth_points(&truth, 0);
    let (x0, y0) = t
        .iter()
        .fold((f64::MAX, f64::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
    let (x1, y1) = t
        .iter()
        .fold((0f64, 0f64), |m, p| (m.0.max(p.0), m.1.max(p.1)));
    let mut e = Engine::default();
    run(
        &mut e,
        &format!("I = imread('{}');", image_path("board_01.png")),
    );
    let mut worst = 0f64;
    for m in [7., 8., 9., 10.] {
        // Top-left crop: the first kept column/row is c0/r0 (1-based).
        let (c0, r0) = (x0.round() - m, y0.round() - m);
        run(
            &mut e,
            &format!("[p, b] = detectCheckerboardPoints(I({r0}:end, {c0}:end));"),
        );
        assert_eq!(values(&mut e, "b"), vec![7., 10.], "top-left crop {m} px");
        let shifted: Vec<(f64, f64)> = t
            .iter()
            .map(|p| (p.0 - (c0 - 1.), p.1 - (r0 - 1.)))
            .collect();
        let nearest = shifted
            .iter()
            .fold(f64::MAX, |d, p| d.min(p.0 - 1.).min(p.1 - 1.));
        assert!((m - 0.5..=m + 0.5).contains(&nearest), "{nearest}");
        let errs = errors(&points(&mut e, "p"), &shifted);
        worst = errs.iter().copied().fold(worst, f64::max);
        // Bottom-right crop: the last kept column/row is c1/r1.
        let (c1, r1) = (x1.round() + m, y1.round() + m);
        run(
            &mut e,
            &format!("[p, b] = detectCheckerboardPoints(I(1:{r1}, 1:{c1}));"),
        );
        assert_eq!(
            values(&mut e, "b"),
            vec![7., 10.],
            "bottom-right crop {m} px"
        );
        let errs = errors(&points(&mut e, "p"), &t);
        worst = errs.iter().copied().fold(worst, f64::max);
    }
    eprintln!("border crops 7..10 px: max {worst:.4}");
    assert!(worst <= 0.35, "max {worst}");
}

#[test]
fn checkerboard_detect_uneven_lighting_keeps_the_board() {
    // board_01 times a left-to-right ramp 0.3..1: cells are compared with the
    // cells next to them, not with the mean over the board.
    let truth = json("truth.json");
    let mut e = Engine::default();
    run(
        &mut e,
        &format!(
            "I = imread('{}'); R = repmat(linspace(0.3, 1, 640), 480, 1); [p, b] = detectCheckerboardPoints(uint8(double(I) .* R));",
            image_path("board_01.png")
        ),
    );
    assert_eq!(values(&mut e, "b"), vec![7., 10.]);
    let errs = errors(&points(&mut e, "p"), &truth_points(&truth, 0));
    let worst = errs.iter().copied().fold(0., f64::max);
    eprintln!("ramp 0.3..1: rms {:.4} max {worst:.4}", rms(&errs));
    assert!(worst <= 0.35, "max {worst}");
}

#[test]
fn checkerboard_detect_upscaled_images_are_found_on_a_reduced_copy() {
    // Fixtures enlarged 4 times (bicubic imresize, 2560x1920): corners are too
    // wide for the full-size search, which returned sub-grids or nothing. The
    // board is found on a reduced copy and refined at full size. imresize maps
    // pixel x (1-based) to 4 (x - 0.5) + 0.5.
    // Bicubic interpolation can shift the edges of board_01. Compare against
    // OpenCV 5.1.0 on MatRust's exact 4x image (tool/refs/calib/detect_upscaled.py).
    const OPENCV_BOARD_01_MAX: f64 = 0.937475461323;
    const OPENCV_BOARD_01_RMS: f64 = 0.651467903435;
    const OPENCV_MARGIN: f64 = 0.1;

    let truth = json("truth.json");
    let mut e = Engine::default();
    for k in [0, 6] {
        let file = truth["images"][k]["file"].as_str().unwrap();
        run(
            &mut e,
            &format!(
                "[p, b] = detectCheckerboardPoints(imresize(imread('{}'), 4));",
                image_path(file)
            ),
        );
        assert_eq!(values(&mut e, "b"), vec![7., 10.], "{file} enlarged");
        let expected: Vec<(f64, f64)> = truth_points(&truth, k)
            .into_iter()
            .map(|p| (4. * (p.0 - 0.5) + 0.5, 4. * (p.1 - 0.5) + 0.5))
            .collect();
        let errs = errors(&points(&mut e, "p"), &expected);
        let worst = errs.iter().copied().fold(0., f64::max);
        let rms = rms(&errs);
        eprintln!("{file} enlarged 4x: rms {rms:.4} max {worst:.4}");
        if k == 0 {
            let max_limit = OPENCV_BOARD_01_MAX + OPENCV_MARGIN;
            let rms_limit = OPENCV_BOARD_01_RMS + OPENCV_MARGIN;
            assert!(
                worst <= max_limit,
                "{file} enlarged: max {worst} > {max_limit}"
            );
            assert!(rms <= rms_limit, "{file} enlarged: rms {rms} > {rms_limit}");
        } else {
            assert!(worst <= 0.5, "{file} enlarged: max {worst} > 0.5");
        }
    }
}

#[test]
fn checkerboard_detect_rejects_scratch_over_budget() {
    let mut e = Engine::default();
    run(&mut e, "I = zeros(3024,4032,'uint8');");
    error_has(&mut e, "detectCheckerboardPoints(I)", "256 MiB");
}
