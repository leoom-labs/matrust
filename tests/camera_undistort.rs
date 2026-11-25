//! Camera calibration piece 7d4: undistortImage and undistortPoints.
//!
//! Expected values: tests/fixtures/calib/undistort_image.json and
//! undistort_points.json written by tool/refs/calib/undistort.py (NumPy
//! float64 source map + scipy.ndimage.map_coordinates, OpenCV 5.1
//! cv2.undistort and cv2.undistortPoints), and properties from the public
//! MathWorks reference pages (output class = input class, 'full' holds the
//! whole undistorted image, 'valid' has no fill pixels). No MATLAB run.
use matrust::{Array, Engine, Matrix, NumericClass, TypedArray};
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

fn error_has(e: &mut Engine, code: &str, part: &str) {
    match e.execute(code) {
        Ok(_) => panic!("{code}\n  succeeded; expected an error"),
        Err(message) => assert!(
            message.contains(part),
            "{code}\n  error {message:?} does not contain {part:?}"
        ),
    }
}

/// Column-major values of an expression (as double).
fn values(e: &mut Engine, expr: &str) -> Vec<f64> {
    run(e, &format!("zz_v = double({expr}); zz_v = zz_v(:)';"));
    matrix(e, "zz_v").data().to_vec()
}

fn truth(e: &mut Engine, expr: &str) -> bool {
    run(e, &format!("zz_b = logical({expr});"));
    let v = values(e, "all(zz_b(:))");
    v == [1.]
}

fn text(e: &mut Engine, expr: &str) -> String {
    run(e, &format!("zz_t = class({expr});"));
    match e.array("zz_t") {
        Some(Array::Character(c)) => String::from_utf16_lossy(c.units()),
        other => panic!("class of {expr}: {other:?}"),
    }
}

fn set(e: &mut Engine, name: &str, rows: usize, cols: usize, data: Vec<f64>) {
    e.variables.insert(
        name.into(),
        Matrix::from_column_vec(rows, cols, data).unwrap(),
    );
}

fn json_vec(v: &Json) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn json_rows(v: &Json) -> Vec<[f64; 2]> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let r = json_vec(r);
            [r[0], r[1]]
        })
        .collect()
}

const IMAGE: &str = "[c, r] = meshgrid(1:640, 1:480); \
    I = 0.5 + 0.2*sin(0.05*c + 0.3) + 0.15*cos(0.07*r).*sin(0.031*c) + 0.1*(r + c)/1120; \
    I8 = uint8(floor(255*I + 0.5)); \
    RGB = cat(3, I, 0.5 + 0.2*sin(0.05*c + 1.3) + 0.15*cos(0.07*r).*sin(0.031*c) + 0.1*(r + c)/1120, \
        0.5 + 0.2*sin(0.05*c + 2.3) + 0.15*cos(0.07*r).*sin(0.031*c) + 0.1*(r + c)/1120);";

fn camera(radial: &[f64], tangential: &[f64]) -> String {
    let f = |v: &[f64]| {
        v.iter()
            .map(|x| format!("{x:e}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!(
        "cam = cameraIntrinsics([800 790], [322 238], [480 640], 'RadialDistortion', [{}], 'TangentialDistortion', [{}]);",
        f(radial),
        f(tangential)
    )
}

fn max_diff(a: &[f64], b: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0., f64::max)
}

/// (a) float64 'same' reference (scipy map_coordinates, linear and nearest),
/// uint8 and RGB; (b) cv2.undistort.
#[test]
fn undistort_image_same_matches_references() {
    let json: Json =
        serde_json::from_str(include_str!("fixtures/calib/undistort_image.json")).unwrap();
    let rows = json_vec(&json["rows"]);
    let cols = json_vec(&json["cols"]);
    let idx: Vec<f64> = rows
        .iter()
        .zip(&cols)
        .map(|(r, c)| r + (c - 1.) * 480.)
        .collect();
    let n = idx.len();
    let mut e = Engine::default();
    run(&mut e, IMAGE);
    set(&mut e, "idx", n, 1, idx);
    for name in ["radial", "full"] {
        let s = &json["sets"][name];
        run(
            &mut e,
            &camera(&json_vec(&s["radial"]), &json_vec(&s["tangential"])),
        );
        let lin = values(&mut e, "undistortImage(I, cam)");
        let lin: Vec<f64> = values_at(&lin, &mut e, "idx");
        let d = max_diff(&lin, &json_vec(&s["linear"]));
        eprintln!("{name}: linear max diff {d:e}");
        assert!(d < 1e-9, "{name}: linear max diff {d:e}");
        let near = values(&mut e, "undistortImage(I, cam, 'nearest')");
        let near = values_at(&near, &mut e, "idx");
        let d = max_diff(&near, &json_vec(&s["nearest"]));
        assert!(d < 1e-9, "{name}: nearest max diff {d:e}");
        // (b) OpenCV fixed-point remap: mean abs diff on the samples.
        let cv = json_vec(&s["opencv"]);
        let mean = lin.iter().zip(&cv).map(|(a, b)| (a - b).abs()).sum::<f64>() / n as f64;
        eprintln!("{name}: opencv mean abs diff {mean:e}");
        assert!(mean < 0.02, "{name}: opencv mean abs diff {mean:e}");
        // uint8: the float sample rounded to the nearest integer.
        assert_eq!(text(&mut e, "undistortImage(I8, cam)"), "uint8");
        let u8v = values(&mut e, "undistortImage(I8, cam)");
        let u8v = values_at(&u8v, &mut e, "idx");
        let d = max_diff(&u8v, &json_vec(&s["uint8_linear"]));
        assert!(d <= 0.5 + 1e-9, "{name}: uint8 max diff {d:e}");
        // RGB: every plane like the gray reference.
        run(&mut e, "J3 = undistortImage(RGB, cam);");
        assert_eq!(values(&mut e, "size(J3)"), [480., 640., 3.]);
        for k in 0..3 {
            let p = values(&mut e, &format!("J3(:, :, {})", k + 1));
            let p = values_at(&p, &mut e, "idx");
            let d = max_diff(&p, &json_vec(&s["rgb_linear"][k]));
            assert!(d < 1e-9, "{name}: rgb plane {k} max diff {d:e}");
        }
    }
}

fn values_at(all: &[f64], e: &mut Engine, idx: &str) -> Vec<f64> {
    matrix(e, idx)
        .data()
        .iter()
        .map(|&i| all[i as usize - 1])
        .collect()
}

#[test]
fn undistort_image_classes_and_intrinsics() {
    let mut e = Engine::default();
    run(&mut e, IMAGE);
    run(&mut e, &camera(&[-0.12, 0.05], &[0., 0.]));
    for (expr, class) in [
        ("I", "double"),
        ("single(I)", "single"),
        ("I8", "uint8"),
        ("uint16(I8)", "uint16"),
        ("I > 0.5", "logical"),
        ("RGB", "double"),
        ("uint8(255*RGB)", "uint8"),
    ] {
        run(&mut e, &format!("J = undistortImage({expr}, cam);"));
        assert_eq!(text(&mut e, "J"), class, "{expr}");
        assert_eq!(
            values(&mut e, "size(J)"),
            values(&mut e, &format!("size({expr})")),
            "{expr}"
        );
    }
    // The second output for 'same': K kept, no distortion.
    run(&mut e, "[J, n] = undistortImage(I, cam);");
    assert_eq!(text(&mut e, "n"), "cameraIntrinsics");
    assert_eq!(values(&mut e, "n.PrincipalPoint"), [322., 238.]);
    assert_eq!(values(&mut e, "n.FocalLength"), [800., 790.]);
    assert_eq!(values(&mut e, "n.ImageSize"), [480., 640.]);
    assert_eq!(values(&mut e, "n.RadialDistortion"), [0., 0.]);
    assert_eq!(values(&mut e, "n.TangentialDistortion"), [0., 0.]);
    // cameraParameters works like its intrinsics.
    run(
        &mut e,
        "p = cameraParameters('K', cam.K, 'RadialDistortion', [-0.12 0.05], 'ImageSize', [480 640]);",
    );
    assert!(truth(
        &mut e,
        "isequal(undistortImage(I, p, 'cubic'), undistortImage(I, cam, 'cubic'))"
    ));
}

#[test]
fn zero_distortion_keeps_the_image() {
    let mut e = Engine::default();
    run(&mut e, IMAGE);
    run(
        &mut e,
        "c0 = cameraIntrinsics([800 790], [322 238], [480 640]);",
    );
    assert!(truth(&mut e, "isequal(undistortImage(I, c0), I)"));
    assert!(truth(&mut e, "isequal(undistortImage(I8, c0), I8)"));
    assert!(truth(
        &mut e,
        "isequal(undistortImage(RGB, c0, 'linear'), RGB)"
    ));
    assert!(truth(
        &mut e,
        "isequal(undistortImage(I, c0, OutputView='full'), I)"
    ));
    assert!(truth(
        &mut e,
        "isequal(undistortImage(I, c0, OutputView='valid'), I)"
    ));
}

/// 'full' contains every source pixel and agrees with 'same' where they
/// overlap; 'valid' has no fill pixels.
#[test]
fn full_and_valid_views() {
    let mut e = Engine::default();
    run(&mut e, IMAGE);
    for (radial, tangential, valid_smaller) in [
        // Barrel lens of the fixtures: the undistorted image is pincushion
        // shaped, so 'same' has no fill and 'valid' is about as large.
        (vec![-0.12, 0.05, 0.01], [1e-3, -5e-4], false),
        // Pincushion lens: 'same' has fill corners, 'full' and 'valid' are
        // smaller than I.
        (vec![0.15, 0.02], [0., 0.], true),
    ] {
        run(&mut e, &camera(&radial, &tangential));
        run(&mut e, "[Js, ns] = undistortImage(I, cam, FillValues=-1);");
        run(
            &mut e,
            "[Jf, nf] = undistortImage(I, cam, OutputView='full', FillValues=-1);",
        );
        run(
            &mut e,
            "[Jv, nv] = undistortImage(I, cam, OutputView='VALID', FillValues=-1);",
        );
        let full = values(&mut e, "size(Jf)");
        let valid = values(&mut e, "size(Jv)");
        // Barrel: the undistorted image is larger than I; pincushion: smaller.
        if valid_smaller {
            assert!(
                full[0] < 480. && full[1] < 640.,
                "{radial:?}: full {full:?}"
            );
        } else {
            assert!(
                full[0] > 480. && full[1] > 640.,
                "{radial:?}: full {full:?}"
            );
        }
        assert_eq!(values(&mut e, "nf.ImageSize"), full);
        assert_eq!(values(&mut e, "nv.ImageSize"), valid);
        // Every border pixel of I, undistorted, lands inside Jf.
        run(
            &mut e,
            "B = [ones(480,1) (1:480)'; 640*ones(480,1) (1:480)'; (1:640)' ones(640,1); (1:640)' 480*ones(640,1)]; \
             q = undistortPoints(B, cam) + repmat(nf.PrincipalPoint - cam.PrincipalPoint, size(B, 1), 1);",
        );
        assert!(truth(
            &mut e,
            "q(:,1) >= 1 & q(:,1) <= size(Jf,2) & q(:,2) >= 1 & q(:,2) <= size(Jf,1)"
        ));
        // Whole-pixel offsets; 'full' equals 'same' on the overlap.
        run(&mut e, "o = cam.PrincipalPoint - nf.PrincipalPoint;");
        assert!(truth(&mut e, "o == round(o)"));
        if valid_smaller {
            // 'full' is inside 'same': Js(1+o(2):..., 1+o(1):...) == Jf.
            assert!(truth(&mut e, "o >= 0"));
            assert!(truth(
                &mut e,
                "isequal(Js(1+o(2):size(Jf,1)+o(2), 1+o(1):size(Jf,2)+o(1)), Jf)"
            ));
        } else {
            assert!(truth(&mut e, "o <= 0"));
            assert!(truth(
                &mut e,
                "isequal(Jf(1-o(2):480-o(2), 1-o(1):640-o(1)), Js)"
            ));
        }
        // 'valid': no fill pixels; 'full' has some.
        assert!(truth(&mut e, "Jv >= 0"));
        assert!(truth(&mut e, "any(Jf(:) == -1)"));
        run(&mut e, "ov = cam.PrincipalPoint - nv.PrincipalPoint;");
        assert!(truth(&mut e, "ov == round(ov)"));
        eprintln!("{radial:?}: full {full:?} valid {valid:?}");
        let n_valid = valid[0] * valid[1];
        if valid_smaller {
            assert!(truth(&mut e, "any(Js(:) == -1)"));
            assert!(
                valid[0] < 480. && valid[1] < 640.,
                "{radial:?}: valid {valid:?}"
            );
        } else {
            assert!(truth(&mut e, "all(Js(:) >= 0)"));
            assert!(n_valid >= 0.95 * 480. * 640., "{radial:?}: valid {valid:?}");
        }
        assert!(n_valid < full[0] * full[1]);
    }
}

/// (c) cv2.undistortPoints and the round trip.
#[test]
fn undistort_points_matches_opencv() {
    let json: Json =
        serde_json::from_str(include_str!("fixtures/calib/undistort_points.json")).unwrap();
    let mut e = Engine::default();
    for name in ["radial", "full"] {
        let s = &json["sets"][name];
        run(
            &mut e,
            &camera(&json_vec(&s["radial"]), &json_vec(&s["tangential"])),
        );
        let d = json_rows(&s["distorted"]);
        let m = d.len();
        assert_eq!(m, 200);
        let data = d
            .iter()
            .map(|p| p[0])
            .chain(d.iter().map(|p| p[1]))
            .collect();
        set(&mut e, "P", m, 2, data);
        run(&mut e, "[U, err] = undistortPoints(P, cam);");
        let u = values(&mut e, "U");
        let col = |rows: &[[f64; 2]]| -> Vec<f64> {
            rows.iter()
                .map(|p| p[0])
                .chain(rows.iter().map(|p| p[1]))
                .collect()
        };
        let cv = max_diff(&u, &col(&json_rows(&s["opencv"])));
        eprintln!("{name}: points vs OpenCV {cv:e} px");
        assert!(cv < 1e-6, "{name}: vs OpenCV {cv:e} px");
        let round = max_diff(&u, &col(&json_rows(&s["truth"])));
        eprintln!("{name}: points round trip {round:e} px");
        assert!(round < 1e-8, "{name}: round trip {round:e} px");
        let errs = values(&mut e, "err");
        assert_eq!(errs.len(), m);
        assert!(
            errs.iter().all(|x| *x < 1e-8),
            "{name}: reprojection errors"
        );
        // Round trip through worldToImage's distortion.
        run(
            &mut e,
            "W = [(U(:,1) - 322)/800, (U(:,2) - 238)/790, zeros(200, 1)]; \
             D = worldToImage(cam, eye(3), [0 0 1], W, 'ApplyDistortion', true);",
        );
        let back = values(&mut e, "D - P");
        assert!(
            back.iter().all(|x| x.abs() < 1e-8),
            "{name}: worldToImage round trip"
        );
    }
    // Class rule: double for double points, otherwise single.
    run(&mut e, "[Us, es] = undistortPoints(single(P), cam);");
    assert_eq!(text(&mut e, "Us"), "single");
    assert_eq!(text(&mut e, "es"), "single");
    assert_eq!(text(&mut e, "undistortPoints(int16(P), cam)"), "single");
    assert_eq!(
        values(&mut e, "size(undistortPoints(zeros(0, 2), cam))"),
        [0., 2.]
    );
}

#[test]
fn undistort_errors() {
    let mut e = Engine::default();
    run(
        &mut e,
        "I = rand(20, 30); cam = cameraIntrinsics([80 80], [15 10], [20 30]);",
    );
    error_has(
        &mut e,
        "undistortImage(I, cam, OutputView='wide')",
        "OutputView must be 'same', 'full' or 'valid'",
    );
    error_has(
        &mut e,
        "undistortImage(I, eye(3))",
        "cameraIntrinsics or cameraParameters",
    );
    error_has(
        &mut e,
        "undistortImage(I)",
        "expected undistortImage(I, intrinsics",
    );
    error_has(
        &mut e,
        "undistortImage(I, cam, Foo=1)",
        "unknown name-value argument 'Foo'",
    );
    error_has(
        &mut e,
        "undistortImage(I, cam, FillValues=[1 2])",
        "FillValues must be a scalar",
    );
    error_has(&mut e, "undistortImage(cam, I)", "undistortImage");
    error_has(
        &mut e,
        "undistortPoints([1 2], 5)",
        "cameraIntrinsics or cameraParameters",
    );
    error_has(&mut e, "undistortPoints([1 2 3], cam)", "M-by-2");
    error_has(
        &mut e,
        "[a, b, c] = undistortPoints([1 2], cam)",
        "Too many output arguments",
    );
}

#[test]
fn undistort_image_over_budget_errors_before_allocating() {
    let mut e = Engine::default();
    // 5000x5000 uint8 (25M px <= 33.5M limit) passes image_basic::image,
    // while its 16 B/px source map alone (50M units) exceeds the 256 MiB budget.
    run(
        &mut e,
        "I = zeros(5000, 5000, 'uint8'); cam = cameraIntrinsics([1500 1500], [2500 2500], [5000 5000], 'RadialDistortion', [0.01 0]);",
    );
    error_has(&mut e, "undistortImage(I, cam)", "256 MiB");
    error_has(
        &mut e,
        "undistortImage(I, cam, OutputView='same')",
        "256 MiB",
    );
    // Also check 'valid' with a 4000x6000 uint8 image whose valid box exceeds 256 MiB.
    let mut e2 = Engine::default();
    let img2 = TypedArray::from_reals(
        NumericClass::Uint8,
        4000,
        6000,
        std::iter::repeat_n(0., 4000 * 6000),
    )
    .unwrap();
    e2.insert_array("I2", Array::Typed(img2)).unwrap();
    run(
        &mut e2,
        "cam2 = cameraIntrinsics([1500 1500], [3000 2000], [4000 6000], 'RadialDistortion', [-0.25 0.03]);",
    );
    error_has(
        &mut e2,
        "undistortImage(I2, cam2, OutputView='valid')",
        "256 MiB",
    );
}

#[test]
fn undistort_points_unsolvable_strong_distortion_is_nan() {
    let mut e = Engine::default();
    run(
        &mut e,
        "cam = cameraIntrinsics([800 790], [322 238], [480 640], 'RadialDistortion', [-0.6 0.05]);",
    );
    run(&mut e, "[U, err] = undistortPoints([760 238], cam);");
    let u = values(&mut e, "U");
    let err = values(&mut e, "err");
    assert!(u.iter().all(|x| x.is_nan()), "expected NaN, got {u:?}");
    assert!(err.iter().all(|x| x.is_nan()), "expected NaN, got {err:?}");

    run(
        &mut e,
        "cam_far = cameraIntrinsics([800 790], [322 238], [480 640], 'RadialDistortion', [0.5 0.3 0.2]);",
    );
    run(&mut e, "[Uf, errf] = undistortPoints([1e7 1e7], cam_far);");
    let uf = values(&mut e, "Uf");
    let errf = values(&mut e, "errf");
    assert!(
        uf.iter().all(|x| x.is_nan() || x.abs() <= 1e6),
        "got {uf:?}"
    );
    assert!(
        uf.iter().all(|x| x.is_nan()),
        "expected NaN for unsolvable far point, got {uf:?}"
    );
    assert!(
        errf.iter().all(|x| x.is_nan()),
        "expected NaN error, got {errf:?}"
    );

    run(
        &mut e,
        "[Uh, errh] = undistortPoints([1e200 1e200], cam_far);",
    );
    let uh = values(&mut e, "Uh");
    let errh = values(&mut e, "errh");
    assert!(uh.iter().all(|x| x.is_nan()), "expected NaN, got {uh:?}");
    assert!(
        errh.iter().all(|x| x.is_nan()),
        "expected NaN, got {errh:?}"
    );
}

#[test]
fn undistort_review_regressions() {
    let mut e = Engine::default();
    run(
        &mut e,
        "cam = cameraIntrinsics([20 20], [16 11], [21 31], 'RadialDistortion', [0.5 0]); J = undistortImage(ones(21,31), cam, OutputView='valid');",
    );
    assert_eq!(values(&mut e, "size(J)"), vec![17., 23.]);
    assert!(truth(&mut e, "all(J(:) == 1)"));
    run(
        &mut e,
        "cam = cameraIntrinsics([1.4999999775000004 1], [1 1], [1 2], 'RadialDistortion', [-1/3 0]); [J, n] = undistortImage([1 2], cam, OutputView='full'); P = undistortPoints([2 1], cam);",
    );
    assert!(truth(
        &mut e,
        "ceil(P(1) + n.PrincipalPoint(1) - cam.PrincipalPoint(1)) == size(J,2)"
    ));
    run(&mut e, "Q = undistortPoints([J(2) 1], cam);");
    assert!(truth(
        &mut e,
        "abs(Q(1) - (2 + cam.PrincipalPoint(1) - n.PrincipalPoint(1))) < 1e-7"
    ));
    run(
        &mut e,
        "cam = cameraIntrinsics([1 1], [1 1], [1 1]); I = bitshift(uint64(1),53)+uint64(1); J = undistortImage(I, cam);",
    );
    assert!(truth(&mut e, "isequal(I,J)"));
    for class in [
        "uint8", "uint16", "uint32", "int8", "int16", "int32", "int64",
    ] {
        run(
            &mut e,
            &format!("I = {class}(7); J = undistortImage(I, cam);"),
        );
        assert!(truth(&mut e, "isequal(I,J)"), "{class}");
    }
    run(
        &mut e,
        "I = bitshift(int64(1),53)+int64(1); J = undistortImage(I, cam);",
    );
    assert!(truth(&mut e, "isequal(I,J)"));
    run(
        &mut e,
        "I = zeros(3072,3072,'uint8'); cam = cameraIntrinsics([2000 2000], [1536 1536], [3072 3072], 'RadialDistortion', [0.01 0]);",
    );
    error_has(&mut e, "undistortImage(I,cam)", "256 MiB");
}
