//! Image processing part 1: image files (truecolor PNG, JPEG, BMP, TIFF),
//! class and range conversion, colour, geometry and filters.
//!
//! Expected values: tests/fixtures/image/*.json from tool/refs/image/make_refs.py
//! (NumPy 2.5, SciPy 1.18, scikit-image 0.26; imresize from a NumPy
//! implementation of Keys 1981 cubic convolution written for the fixture), and
//! hand values worked from the published formulas below. No MATLAB evidence;
//! GNU Octave was not available where the fixtures were generated.
//!
//! imrotate convention: MatRust turns counterclockwise about the image centre
//! ((w+1)/2, (h+1)/2), as scikit-image `rotate` does about ((cols-1)/2,
//! (rows-1)/2) in 0-based pixels, so the 'crop' fixtures come straight from
//! `rotate(resize=False)`. For 'loose' MatRust makes the output
//! ceil(w|cos|+h|sin|) by ceil(w|sin|+h|cos|) pixels (the rotated pixel-edge
//! rectangle, as the reference page asks: large enough for the whole image)
//! while scikit-image's `resize=True` measures between pixel centres; the loose
//! fixtures therefore use scikit-image `warp` with the same rotation about the
//! two centres. The 6x8 test image has no nearest-neighbour sample on an exact
//! .5 tie at 30 degrees, so rounding noise cannot change the expected values.
//!
//! Tolerances: integer results and order statistics exact; double results
//! 1e-12 relative (absolute below 1); rotation 1e-9 (scikit-image works in its
//! own affine matrix arithmetic).
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

fn class_is(e: &mut Engine, expr: &str, class: &str) -> bool {
    run(
        e,
        &format!("zz_c = double(strcmp(class({expr}), '{class}'));"),
    );
    matrix(e, "zz_c").data() == [1.]
}

fn assert_values(e: &mut Engine, expr: &str, dims: &[usize], expected: &[f64], tol: f64) {
    let (d, v) = values(e, expr);
    assert_eq!(d, dims, "{expr}: size");
    for (i, (a, b)) in v.iter().zip(expected).enumerate() {
        let ok = if tol == 0. {
            a == b || a.is_nan() && b.is_nan()
        } else {
            (a - b).abs() <= tol * b.abs().max(1.)
        };
        assert!(ok, "{expr}: element {i} is {a}, expected {b}");
    }
}

fn literal(input: &Json) -> String {
    let class = input["class"].as_str().unwrap();
    let dims: Vec<String> = input["dims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_u64().unwrap().to_string())
        .collect();
    let data: Vec<String> = input["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| format!("{:?}", v.as_f64().unwrap()))
        .collect();
    let body = format!("reshape([{}], [{}])", data.join(" "), dims.join(" "));
    if class == "double" {
        body
    } else {
        format!("{class}({body})")
    }
}

/// Runs every case of one fixture file; returns the largest error seen.
fn fixture(text: &str) -> f64 {
    let json: Json = serde_json::from_str(text).unwrap();
    let mut worst = 0f64;
    for case in json["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut e = Engine::default();
        for (var, input) in case["inputs"].as_object().unwrap() {
            run(&mut e, &format!("{var} = {};", literal(input)));
        }
        let expr = case["expr"].as_str().unwrap();
        run(&mut e, &format!("x = {expr};"));
        let expected = &case["expected"];
        let class = expected["class"].as_str().unwrap();
        assert!(
            class_is(&mut e, "x", class),
            "{name}: {expr} is not {class}"
        );
        let dims: Vec<usize> = expected["dims"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d.as_u64().unwrap() as usize)
            .collect();
        let want: Vec<f64> = expected["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let tol = case["tol"].as_f64().unwrap();
        let (d, got) = values(&mut e, "x");
        assert_eq!(d, dims, "{name}: {expr} size");
        for (i, (a, b)) in got.iter().zip(&want).enumerate() {
            let err = (a - b).abs();
            worst = worst.max(err);
            let ok = if tol == 0. {
                a == b
            } else {
                err <= tol * b.abs().max(1.)
            };
            assert!(ok, "{name}: {expr} element {i} is {a}, expected {b}");
        }
    }
    eprintln!(
        "{}: largest abs error {worst:e}",
        json["cases"][0]["name"].as_str().unwrap()
    );
    worst
}

// ------------------------------------------------------------ reference files

#[test]
fn imresize_matches_the_keys_reference() {
    let worst = fixture(include_str!("fixtures/image/imresize.json"));
    assert!(worst < 1e-12, "{worst}");
}

#[test]
fn imrotate_matches_scikit_image() {
    let worst = fixture(include_str!("fixtures/image/imrotate.json"));
    assert!(worst < 1e-9, "{worst}");
}

#[test]
fn imfilter_matches_scipy_for_every_boundary() {
    let worst = fixture(include_str!("fixtures/image/imfilter.json"));
    assert!(worst < 1e-11, "{worst}");
}

#[test]
fn imgaussfilt_matches_the_numpy_kernel() {
    let worst = fixture(include_str!("fixtures/image/imgaussfilt.json"));
    assert!(worst < 1e-12, "{worst}");
}

#[test]
fn medfilt2_matches_scipy_and_the_even_rule() {
    let worst = fixture(include_str!("fixtures/image/medfilt2.json"));
    assert_eq!(worst, 0.);
}

#[test]
fn colour_conversions_match_numpy_and_colorsys() {
    let worst = fixture(include_str!("fixtures/image/color.json"));
    assert!(worst < 1e-12, "{worst}");
}

// --------------------------------------------------------------------- files

fn in_folder() -> (Engine, tempfile::TempDir) {
    let dir = tempfile::Builder::new()
        .prefix("image_part1")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap();
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    (e, dir)
}

#[test]
fn truecolor_png_round_trips() {
    let (mut e, _dir) = in_folder();
    run(
        &mut e,
        "A = uint8(cat(3, [0 255 7; 128 64 9], [10 20 30; 40 50 60], [255 0 1; 2 3 4]));
         imwrite(A, 'rgb.png'); B = imread('rgb.png');
         a = uint8([0 50 100; 150 200 255]); imwrite(A, 'rgba.png', 'Alpha', a);
         [C, map, alpha] = imread('rgba.png');
         W = uint16(cat(3, [0 65535; 1000 2], [3 4; 5 6], [7 8; 9 60000]));
         imwrite(W, 'w16.png'); D = imread('w16.png');
         G = uint8(magic(4)); imwrite(G, 'g.png'); [G2, gmap, galpha] = imread('g.png');
         imwrite(G, 'ga.png', 'Alpha', uint8([1 2 3 4; 5 6 7 8; 9 10 11 12; 13 14 15 16]));
         [G3, ~, ga] = imread('ga.png');
         imwrite(cat(3, [0 0.5; 1 2], [0.25 0; 0 0], [1 1; NaN 0]), 'd.png'); dd = imread('d.png');
         imwrite(cat(3, true(2), false(2), true(2)), 'l.png'); ll = imread('l.png');",
    );
    for (expr, class) in [
        ("B", "uint8"),
        ("C", "uint8"),
        ("D", "uint16"),
        ("G2", "uint8"),
    ] {
        assert!(class_is(&mut e, expr, class), "{expr}");
    }
    run(
        &mut e,
        "ok = [isequal(A, B), isequal(A, C), isempty(map), isequal(alpha, a), isequal(W, D), ...
               isequal(G, G2), isempty(gmap), isempty(galpha), isequal(G3, G), isequal(size(ga), [4 4])];",
    );
    assert_values(&mut e, "ok", &[1, 10], &[1.; 10], 0.);
    assert_values(&mut e, "ga(:, 1)'", &[1, 4], &[1., 5., 9., 13.], 0.);
    // Double data: [0, 1] scaled to 255 and rounded, NaN as the top level (as for gray PNG).
    assert_values(
        &mut e,
        "dd",
        &[2, 2, 3],
        &[0., 255., 128., 255., 64., 0., 0., 0., 255., 255., 255., 0.],
        0.,
    );
    assert_values(&mut e, "ll(:, :, 2)", &[2, 2], &[0.; 4], 0.);
    assert_values(&mut e, "ll(:, :, 3)", &[2, 2], &[255.; 4], 0.);
    run(
        &mut e,
        "i = imfinfo('rgba.png'); f = {i.BitDepth, i.ColorType};",
    );
    run(
        &mut e,
        "bd = i.BitDepth; ct = double(strcmp(i.ColorType, 'truecolor'));",
    );
    assert_eq!(matrix(&e, "bd").data(), [32.]);
    assert_eq!(matrix(&e, "ct").data(), [1.]);
}

#[test]
#[cfg(feature = "plot-export")]
fn jpeg_bmp_and_tiff_round_trips() {
    let (mut e, _dir) = in_folder();
    run(
        &mut e,
        "[x, y] = meshgrid(0:31, 0:23);
         S = uint8(cat(3, 4*x + 2*y, 200 - 3*y, 60 + 2*x + y));
         imwrite(S, 's.jpg'); J = imread('s.jpg');
         jerr = mean(abs(double(J(:)) - double(S(:))));
         imwrite(S, 'q.jpg', 'Quality', 95); Q = imread('q.jpg'); jq = mean(abs(double(Q(:)) - double(S(:))));
         imwrite(S(:, :, 1), 'g.jpg'); Jg = imread('g.jpg');
         S1 = S(:, :, 1); gerr = mean(abs(double(Jg(:)) - double(S1(:))));
         imwrite(S, 's.bmp'); Bm = imread('s.bmp');
         imwrite(S(:, :, 2), 'g.bmp'); [Bg, Bmap] = imread('g.bmp');
         imwrite(uint8([0 1; 2 1]), [1 0 0; 0 1 0; 0 0 1], 'p.bmp'); [Px, Pmap] = imread('p.bmp');
         imwrite(S, 's.tif'); T = imread('s.tif');
         W = uint16(cat(3, [0 65535; 1000 2], [3 4; 5 6], [7 8; 9 60000])); imwrite(W, 'w.tif'); Tw = imread('w.tif');
         imwrite(uint16(magic(4) * 1000), 'g.tif'); Tg = imread('g.tif');",
    );
    let jerr = matrix(&e, "jerr").data()[0];
    let jq = matrix(&e, "jq").data()[0];
    let gerr = matrix(&e, "gerr").data()[0];
    assert!(jerr < 3., "JPEG quality 75 mean abs error {jerr}");
    assert!(
        jq < 3. && jq <= jerr + 1e-12,
        "JPEG quality 95 mean abs error {jq}"
    );
    assert!(gerr < 3., "gray JPEG mean abs error {gerr}");
    for (expr, class) in [
        ("J", "uint8"),
        ("Bm", "uint8"),
        ("T", "uint8"),
        ("Tw", "uint16"),
        ("Tg", "uint16"),
    ] {
        assert!(class_is(&mut e, expr, class), "{expr}");
    }
    run(
        &mut e,
        "ok = [isequal(size(J), [24 32 3]), isequal(Bm, S), isequal(Bg, S(:, :, 2)), isequal(Bmap(1:3, :), [0 0 0; 1 1 1; 2 2 2] / 255), ...
               isequal(Px, uint8([0 1; 2 1])), isequal(Pmap, [1 0 0; 0 1 0; 0 0 1]), ...
               isequal(T, S), isequal(Tw, W), isequal(Tg, uint16(magic(4) * 1000))];",
    );
    assert_values(&mut e, "ok", &[1, 9], &[1.; 9], 0.);
    // imfinfo: the common fields for every format.
    for (file, format, width, height, depth, color) in [
        ("s.jpg", "jpg", 32., 24., 24., "truecolor"),
        ("g.jpg", "jpg", 32., 24., 8., "grayscale"),
        ("s.bmp", "bmp", 32., 24., 24., "truecolor"),
        ("g.bmp", "bmp", 32., 24., 8., "indexed"),
        ("s.tif", "tif", 32., 24., 24., "truecolor"),
        ("g.tif", "tif", 4., 4., 16., "grayscale"),
    ] {
        run(
            &mut e,
            &format!(
                "i = imfinfo('{file}'); v = [i.Width, i.Height, i.BitDepth, i.FileSize > 0, strcmp(i.Format, '{format}'), strcmp(i.ColorType, '{color}'), contains(i.Filename, '{file}')];"
            ),
        );
        assert_eq!(
            matrix(&e, "v").data(),
            [width, height, depth, 1., 1., 1., 1.],
            "{file}"
        );
    }
    run(&mut e, "i = imfinfo('s.jpg'); n = i.NumberOfSamples;");
    assert_eq!(matrix(&e, "n").data(), [3.]);
}

// --------------------------------------------------------- class and range

#[test]
fn class_converters_follow_the_documented_scalings() {
    let mut e = Engine::default();
    // (expression, class, values): uint8 / 255, uint16 / 65535, int16 offset
    // by 32768, logical true to the class maximum, doubles clipped to [0, 1].
    let cases: &[(&str, &str, &[f64])] = &[
        ("im2double(uint8([0 51 255]))", "double", &[0., 0.2, 1.]),
        (
            "im2double(uint16([0 13107 65535]))",
            "double",
            &[0., 0.2, 1.],
        ),
        ("im2double(int16([-32768 32767]))", "double", &[0., 1.]),
        ("im2double(logical([1 0]))", "double", &[1., 0.]),
        ("im2double([-0.5 2])", "double", &[-0.5, 2.]),
        ("im2double(uint8([0 4]), 'indexed')", "double", &[1., 5.]),
        ("im2single(uint8([0 255]))", "single", &[0., 1.]),
        (
            "im2uint8([-1 0 0.5 1 2 NaN])",
            "uint8",
            &[0., 0., 128., 255., 255., 0.],
        ),
        ("im2uint8(single([0.2 0.998]))", "uint8", &[51., 254.]),
        (
            "im2uint8(uint16([0 257 65535 128]))",
            "uint8",
            &[0., 1., 255., 0.],
        ),
        (
            "im2uint8(int16([-32768 0 32767]))",
            "uint8",
            &[0., 128., 255.],
        ),
        ("im2uint8(logical([1 0]))", "uint8", &[255., 0.]),
        ("im2uint8(uint8([3 250]))", "uint8", &[3., 250.]),
        ("im2uint8([1 256], 'indexed')", "uint8", &[0., 255.]),
        (
            "im2uint16([0 0.5 1 7])",
            "uint16",
            &[0., 32768., 65535., 65535.],
        ),
        ("im2uint16(uint8([0 1 255]))", "uint16", &[0., 257., 65535.]),
        ("im2uint16(int16([-32768 0]))", "uint16", &[0., 32768.]),
        ("im2uint16(true)", "uint16", &[65535.]),
        ("mat2gray([2 4 6 10])", "double", &[0., 0.25, 0.5, 1.]),
        ("mat2gray([2 4 6 10], [4 6])", "double", &[0., 0., 1., 1.]),
        ("mat2gray(uint8([10 20 30]))", "double", &[0., 0.5, 1.]),
        ("mat2gray([3 3 3])", "double", &[1., 1., 1.]),
        ("imcomplement([0 0.25 1])", "double", &[1., 0.75, 0.]),
        (
            "imcomplement(uint8([0 55 255]))",
            "uint8",
            &[255., 200., 0.],
        ),
        ("imcomplement(uint16(1000))", "uint16", &[64535.]),
        (
            "imcomplement(int16([-32768 0 100]))",
            "int16",
            &[32767., -1., -101.],
        ),
        ("imcomplement(logical([1 0]))", "logical", &[0., 1.]),
    ];
    for (expr, class, want) in cases {
        assert!(class_is(&mut e, expr, class), "{expr} is not {class}");
        let (_, got) = values(&mut e, expr);
        assert_eq!(&got, want, "{expr}");
    }
    // N-D: im2double of an RGB uint8 image keeps its size.
    assert_values(
        &mut e,
        "size(im2double(zeros(2, 3, 3, 'uint8')))",
        &[1, 3],
        &[2., 3., 3.],
        0.,
    );
}

// -------------------------------------------------------------------- colour

#[test]
fn rgb2gray_ind2rgb_and_hsv_by_hand() {
    let mut e = Engine::default();
    // 0.298936021293775 * 255 = 76.229 -> 76; 0.587043... * 255 = 149.70 -> 150;
    // white: (0.298936... + 0.587043... + 0.114020...) * 255 = 255.
    assert_values(
        &mut e,
        "rgb2gray(uint8(cat(3, [255 0 0 255], [0 255 0 255], [0 0 255 255])))",
        &[1, 4],
        &[76., 150., 29., 255.],
        0.,
    );
    assert!(class_is(
        &mut e,
        "rgb2gray(uint16(ones(2, 2, 3)))",
        "uint16"
    ));
    assert_values(
        &mut e,
        "ind2rgb(uint8([0 2]), [0 0 0; 0.5 0.5 0.5; 1 0 0])",
        &[1, 2, 3],
        &[0., 1., 0., 0., 0., 0.],
        0.,
    );
    // hsv2rgb(rgb2hsv(x)) gives x back.
    run(
        &mut e,
        "x = reshape(mod((1:60) * 0.618034, 1), 4, 5, 3); y = hsv2rgb(rgb2hsv(x)); d = max(abs(y(:) - x(:)));
         u = uint8(reshape(mod((1:27) * 37, 256), 3, 3, 3)); hu = rgb2hsv(u); su = hsv2rgb(hu) * 255;
         du = max(abs(su(:) - double(u(:))));",
    );
    assert!(matrix(&e, "d").data()[0] < 1e-12);
    assert!(matrix(&e, "du").data()[0] < 1e-12);
    assert!(class_is(&mut e, "hu", "double"));
    assert!(class_is(&mut e, "rgb2hsv(single(x))", "single"));
}

// ------------------------------------------------------------------ geometry

#[test]
fn imresize_scale_one_and_aspect_rule() {
    let mut e = Engine::default();
    run(
        &mut e,
        "A = uint8(magic(5)); same = double(isequal(imresize(A, 1), A));
         B = reshape(1:32, 4, 8); s = size(imresize(B, [NaN 6])); t = size(imresize(B, [2 NaN]));
         r = size(imresize(zeros(5, 7, 3), 0.5)); l = class(imresize(true(4), 2));",
    );
    assert_eq!(matrix(&e, "same").data(), [1.]);
    // [NaN 6]: rows ceil(4 * 6/8) = 3; [2 NaN]: columns ceil(8 * 2/4) = 4.
    assert_eq!(matrix(&e, "s").data(), [3., 6.]);
    assert_eq!(matrix(&e, "t").data(), [2., 4.]);
    assert_eq!(matrix(&e, "r").data(), [3., 4., 3.]);
    assert!(class_is(&mut e, "imresize(true(4), 2)", "logical"));
}

#[test]
fn imrotate_quarter_turns_equal_rot90() {
    let mut e = Engine::default();
    run(
        &mut e,
        "A = reshape(1:12, 3, 4); R = cat(3, A, 2 * A, 3 * A);",
    );
    for (angle, k) in [(90, 1), (180, 2), (270, 3), (-90, 3), (450, 1)] {
        run(
            &mut e,
            &format!(
                "ok = double(isequal(imrotate(A, {angle}), rot90(A, {k})) && isequal(imrotate(A, {angle}, 'bilinear'), rot90(A, {k})));
                 Q = imrotate(R, {angle}); okr = double(isequal(Q(:, :, 2), rot90(2 * A, {k})));"
            ),
        );
        assert_eq!(matrix(&e, "ok").data(), [1.], "angle {angle}");
        assert_eq!(matrix(&e, "okr").data(), [1.], "RGB angle {angle}");
    }
    run(
        &mut e,
        "s = size(imrotate(ones(6, 8), 30)); c = size(imrotate(ones(6, 8), 30, 'crop'));",
    );
    // ceil(8 sin 30 + 6 cos 30) = ceil(9.196) = 10; ceil(8 cos 30 + 6 sin 30) = ceil(9.928) = 10.
    assert_eq!(matrix(&e, "s").data(), [10., 10.]);
    assert_eq!(matrix(&e, "c").data(), [6., 8.]);
    assert!(class_is(
        &mut e,
        "imrotate(uint8(magic(4)), 30, 'bilinear')",
        "uint8"
    ));
}

#[test]
fn imcrop_and_imtranslate() {
    let mut e = Engine::default();
    run(&mut e, "M = reshape(1:42, 6, 7);");
    // [xmin ymin width height] = [2 3 3 2]: columns 2..5, rows 3..5 (width+1, height+1).
    assert_values(
        &mut e,
        "imcrop(M, [2 3 3 2])",
        &[3, 4],
        &[9., 10., 11., 15., 16., 17., 21., 22., 23., 27., 28., 29.],
        0.,
    );
    // Clipped at the border; a rectangle outside gives an empty result.
    assert_values(
        &mut e,
        "size(imcrop(M, [6 5 10 10]))",
        &[1, 2],
        &[2., 2.],
        0.,
    );
    assert_values(
        &mut e,
        "size(imcrop(M, [20 20 2 2]))",
        &[1, 2],
        &[0., 0.],
        0.,
    );
    assert_values(
        &mut e,
        "size(imcrop(zeros(6, 7, 3), [1 1 2 2]))",
        &[1, 3],
        &[3., 3., 3.],
        0.,
    );
    // Integer shift: right by 1, down by 2, zero fill.
    assert_values(
        &mut e,
        "imtranslate([1 2 3; 4 5 6; 7 8 9], [1 2])",
        &[3, 3],
        &[0., 0., 0., 0., 0., 1., 0., 0., 2.],
        0.,
    );
    // Fractional shift: bilinear between neighbours, 0 outside.
    assert_values(
        &mut e,
        "imtranslate([2 4 8], [0.5 0])",
        &[1, 3],
        &[1., 3., 6.],
        0.,
    );
    assert_values(
        &mut e,
        "imtranslate(uint8([10 20 30]), [-0.25 0])",
        &[1, 3],
        &[13., 23., 23.],
        0.,
    );
}

// ------------------------------------------------------------------- filters

#[test]
fn fspecial_kernels_follow_the_published_formulas() {
    let mut e = Engine::default();
    assert_values(&mut e, "fspecial('average')", &[3, 3], &[1. / 9.; 9], 1e-15);
    assert_values(
        &mut e,
        "fspecial('average', [1 4])",
        &[1, 4],
        &[0.25; 4],
        0.,
    );
    // Gaussian 3x3, sigma 0.5: exp(-r^2/0.5) normalised.
    let g = |r2: f64| (-r2 / 0.5).exp();
    let s = g(0.) + 4. * g(1.) + 4. * g(2.);
    let (c, e1, d) = (g(0.) / s, g(1.) / s, g(2.) / s);
    assert_values(
        &mut e,
        "fspecial('gaussian')",
        &[3, 3],
        &[d, e1, d, e1, c, e1, d, e1, d],
        1e-15,
    );
    // Laplacian alpha 0.2: 4/(alpha+1) * [a/4 (1-a)/4 a/4; (1-a)/4 -1 ...].
    let (a, b) = (4. / 1.2 * 0.05, 4. / 1.2 * 0.2);
    assert_values(
        &mut e,
        "fspecial('laplacian')",
        &[3, 3],
        &[a, b, a, b, -4. / 1.2, b, a, b, a],
        1e-15,
    );
    assert_values(
        &mut e,
        "fspecial('laplacian', 0)",
        &[3, 3],
        &[0., 1., 0., 1., -4., 1., 0., 1., 0.],
        0.,
    );
    // Unsharp alpha 0.2: 1/(alpha+1) * [-a a-1 -a; a-1 a+5 a-1; -a a-1 -a].
    let (p, q, r) = (-0.2 / 1.2, -0.8 / 1.2, 5.2 / 1.2);
    assert_values(
        &mut e,
        "fspecial('unsharp')",
        &[3, 3],
        &[p, q, p, q, r, q, p, q, p],
        1e-15,
    );
    assert_values(
        &mut e,
        "fspecial('sobel')",
        &[3, 3],
        &[1., 0., -1., 2., 0., -2., 1., 0., -1.],
        0.,
    );
    assert_values(
        &mut e,
        "fspecial('prewitt')",
        &[3, 3],
        &[1., 0., -1., 1., 0., -1., 1., 0., -1.],
        0.,
    );
    // LoG 3x3, sigma 1: (r^2 - 2) exp(-r^2/2) / sum(exp(-r^2/2)), shifted to sum 0.
    let h = |r2: f64| (-r2 / 2.).exp();
    let sum = h(0.) + 4. * h(1.) + 4. * h(2.);
    let raw = [
        (2. - 2.) * h(2.) / sum,
        (1. - 2.) * h(1.) / sum,
        (0. - 2.) * h(0.) / sum,
    ];
    let mean = (4. * raw[0] + 4. * raw[1] + raw[2]) / 9.;
    let (k0, k1, k2) = (raw[0] - mean, raw[1] - mean, raw[2] - mean);
    assert_values(
        &mut e,
        "fspecial('log', 3, 1)",
        &[3, 3],
        &[k0, k1, k0, k1, k2, k1, k0, k1, k0],
        1e-14,
    );
    run(&mut e, "z = abs(sum(sum(fspecial('log'))));");
    assert!(matrix(&e, "z").data()[0] < 1e-15);
    assert!(
        error(&mut e, "fspecial('disk')")
            .contains("fspecial: the filter type 'disk' is not supported yet")
    );
    assert!(error(&mut e, "fspecial('motion', 9, 0)").contains("not supported yet"));
}

#[test]
fn imfilter_keeps_the_class_and_saturates() {
    let mut e = Engine::default();
    // 200 * 2 = 400 saturates to 255; -50 saturates to 0; 2.5 rounds to 3.
    assert_values(
        &mut e,
        "imfilter(uint8([200 50 5]), [2])",
        &[1, 3],
        &[255., 100., 10.],
        0.,
    );
    assert_values(
        &mut e,
        "imfilter(uint8([10 60 5]), [0 0.5 -1])",
        &[1, 3],
        &[0., 25., 3.],
        0.,
    );
    assert!(class_is(&mut e, "imfilter(single([1 2]), 1)", "single"));
    assert!(class_is(&mut e, "imfilter(int16([1 2]), 1)", "int16"));
    assert_values(
        &mut e,
        "size(imfilter(ones(4, 5, 3), ones(2, 3), 'full'))",
        &[1, 3],
        &[5., 7., 3.],
        0.,
    );
    // medfilt2 default: 3x3 with zero padding.
    assert_values(
        &mut e,
        "medfilt2([1 9 3; 4 5 6; 7 8 2])",
        &[3, 3],
        &[0., 4., 0., 3., 5., 4., 0., 3., 0.],
        0.,
    );
    // Reference page example for an even window: [1 5; 4 8] gives 4 (uint8, fraction dropped).
    assert_values(
        &mut e,
        "medfilt2(uint8([1 5; 4 8]), [2 2])",
        &[2, 2],
        &[4., 2., 2., 0.],
        0.,
    );
}

// -------------------------------------------------------------------- errors

#[test]
fn errors_name_the_problem() {
    let mut e = Engine::default();
    // Wrong class.
    assert!(
        error(&mut e, "im2uint8(int32(5))").contains(
            "im2uint8: expected an image of class double, single, logical, uint8, uint16 or int16, not int32"
        )
    );
    assert!(
        error(&mut e, "rgb2gray(int8(ones(2, 2, 3)))")
            .contains("rgb2gray: expected double, single, uint8 or uint16 input, not int8")
    );
    assert!(
        error(&mut e, "imresize({1}, 2)")
            .contains("imresize: expected a numeric or logical image, not cell")
    );
    // 4-D input.
    assert!(
        error(&mut e, "imresize(ones(2, 2, 2, 2), 2)")
            .contains("imresize: expected a 2-D image or an H-by-W-by-C array, not a 4-D array")
    );
    assert!(
        error(&mut e, "imfilter(ones(2, 2, 2, 2), 1)")
            .contains("imfilter: expected a 2-D image or an H-by-W-by-C array, not a 4-D array")
    );
    // Bad option names.
    assert!(
        error(&mut e, "imfilter(ones(3), 1, 'bogus')").contains("imfilter: unknown option 'bogus'")
    );
    assert!(
        error(&mut e, "imresize(ones(3), 2, 'Bogus', 1)")
            .contains("imresize: unknown option 'Bogus'")
    );
    assert!(
        error(&mut e, "imrotate(ones(3), 30, 'sideways')")
            .contains("imrotate: unknown option 'sideways'")
    );
    assert!(
        error(&mut e, "imgaussfilt(ones(3), 1, 'Pad', 1)")
            .contains("imgaussfilt: unknown option 'Pad'")
    );
    assert!(
        error(&mut e, "medfilt2(ones(3), 'mirror')")
            .contains("medfilt2: unknown padding option 'mirror'")
    );
    assert!(error(&mut e, "imcrop(ones(3))").contains("imcrop needs a rectangle"));
    assert!(error(&mut e, "medfilt2(ones(3, 3, 3))").contains("medfilt2: expected a 2-D image"));
    assert!(
        error(&mut e, "ind2rgb(logical([1 0]), [0 0 0; 1 1 1])")
            .contains("ind2rgb: X must be single, double, uint8 or uint16, not logical")
    );
    let (mut f, _dir) = in_folder();
    assert!(
        error(&mut f, "imwrite(uint8(ones(2, 2, 2)), 'two.png')")
            .contains("PNG image data must be either MxN or MxNx3.")
    );
    #[cfg(feature = "plot-export")]
    assert!(
        error(&mut f, "imwrite(uint8(ones(2, 2, 4)), 'four.jpg')")
            .contains("JPEG image data must be either MxN or MxNx3.")
    );
    #[cfg(feature = "plot-export")]
    assert!(
        error(&mut f, "imwrite(uint8(1), 'x.tif', 'Compression', 'none')")
            .contains("imwrite: the TIFF parameter 'compression' is not supported yet")
    );
}

// ------------------------------------------------------- fix round 1 checks

/// A little-endian TIFF header with one IFD: width 2, height 2 and a
/// BitsPerSample SHORT tag with `bits_count` values (8 each, in the entry).
fn tiff_header(bits_count: u32) -> Vec<u8> {
    let mut b = b"II*\0".to_vec();
    b.extend(8u32.to_le_bytes());
    b.extend(3u16.to_le_bytes());
    for (tag, count, value) in [(256u16, 1u32, 2u16), (257, 1, 2), (258, bits_count, 8)] {
        b.extend(tag.to_le_bytes());
        b.extend(3u16.to_le_bytes());
        b.extend(count.to_le_bytes());
        b.extend(value.to_le_bytes());
        b.extend([0, 0]);
    }
    b.extend(0u32.to_le_bytes());
    b
}

/// A 2x2 8-bit BMP header (40-byte info header) whose palette claims `used`
/// entries; `present` 4-byte entries follow, then 8 bytes of pixel rows.
fn bmp_header(used: u32, present: usize) -> Vec<u8> {
    let mut info = Vec::new();
    for v in [40u32, 2, 2] {
        info.extend(v.to_le_bytes());
    }
    info.extend(1u16.to_le_bytes());
    info.extend(8u16.to_le_bytes());
    for v in [0u32, 8, 2835, 2835, used, 0] {
        info.extend(v.to_le_bytes());
    }
    let offset = 14 + info.len() + 4 * present;
    let mut b = b"BM".to_vec();
    b.extend(((offset + 8) as u32).to_le_bytes());
    b.extend([0; 4]);
    b.extend((offset as u32).to_le_bytes());
    b.extend(info);
    for k in 0..present {
        b.extend([k as u8, k as u8, k as u8, 0]);
    }
    b.extend([0, 1, 0, 0, 1, 0, 0, 0]);
    b
}

#[test]
fn imfinfo_rejects_damaged_headers_without_panicking() {
    let (mut e, dir) = in_folder();
    // Control: the same header with one BitsPerSample value reads as 8-bit.
    std::fs::write(dir.path().join("one.tif"), tiff_header(1)).unwrap();
    run(
        &mut e,
        "i = imfinfo('one.tif'); v = [i.Width, i.Height, i.BitDepth];",
    );
    assert_eq!(matrix(&e, "v").data(), [2., 2., 8.]);
    // BitsPerSample with count 0 names no depth: an error, not a panic.
    std::fs::write(dir.path().join("zero.tif"), tiff_header(0)).unwrap();
    assert!(
        error(&mut e, "imfinfo('zero.tif')").contains("TIFF file is truncated or damaged."),
        "count-0 BitsPerSample"
    );
    // Control: a 2-entry palette that is present in the file.
    std::fs::write(dir.path().join("two.bmp"), bmp_header(2, 2)).unwrap();
    run(
        &mut e,
        "i = imfinfo('two.bmp'); n = i.NumColormapEntries; m = i.Colormap;",
    );
    assert_eq!(matrix(&e, "n").data(), [2.]);
    assert_eq!(
        matrix(&e, "m").data(),
        [0., 1. / 255., 0., 1. / 255., 0., 1. / 255.]
    );
    // A palette of 2^32 - 1 entries cannot fit in the file: an error before
    // anything is allocated for it.
    std::fs::write(dir.path().join("huge.bmp"), bmp_header(u32::MAX, 2)).unwrap();
    assert!(
        error(&mut e, "imfinfo('huge.bmp')").contains("BMP file is truncated or damaged."),
        "2^32-1 palette entries"
    );
}

#[test]
fn imrotate_quarter_turn_crop_is_a_centred_rot90_window() {
    let mut e = Engine::default();
    run(
        &mut e,
        "A = reshape(1:12, 3, 4); T = reshape(1:12, 4, 3); W = reshape(1:15, 3, 5);",
    );
    // rot90(A) = [10 11 12; 7 8 9; 4 5 6; 1 2 3] (4x3) cut and padded to 3x4:
    // the odd extra row goes from the bottom, the odd zero column on the left.
    let ccw: [f64; 12] = [0., 0., 0., 10., 7., 4., 11., 8., 5., 12., 9., 6.];
    // rot90(A, 3) = [3 2 1; 6 5 4; 9 8 7; 12 11 10] by the same rule.
    let cw: [f64; 12] = [0., 0., 0., 3., 6., 9., 2., 5., 8., 1., 4., 7.];
    for method in ["nearest", "bilinear"] {
        for (angle, expected) in [
            (90, &ccw),
            (450, &ccw),
            (-270, &ccw),
            (270, &cw),
            (-90, &cw),
        ] {
            assert_values(
                &mut e,
                &format!("imrotate(A, {angle}, '{method}', 'crop')"),
                &[3, 4],
                expected,
                0.,
            );
        }
        // rot90(T) = [9 10 11 12; 5 6 7 8; 1 2 3 4] (3x4) to 4x3: the odd zero
        // row on top, the odd extra column cut from the right.
        assert_values(
            &mut e,
            &format!("imrotate(T, 90, '{method}', 'crop')"),
            &[4, 3],
            &[0., 9., 5., 1., 0., 10., 6., 2., 0., 11., 7., 3.],
            0.,
        );
        // An even difference is split evenly: one zero column on each side.
        assert_values(
            &mut e,
            &format!("imrotate(W, 90, '{method}', 'crop')"),
            &[3, 5],
            &[
                0., 0., 0., 10., 7., 4., 11., 8., 5., 12., 9., 6., 0., 0., 0.,
            ],
            0.,
        );
        // Half turns keep the size: exactly rot90(A, 2).
        run(
            &mut e,
            &format!("ok = double(isequal(imrotate(A, 180, '{method}', 'crop'), rot90(A, 2)));"),
        );
        assert_eq!(matrix(&e, "ok").data(), [1.], "{method} 180 crop");
    }
    // Every plane of an RGB image, and the class is kept.
    run(
        &mut e,
        "R = imrotate(uint8(cat(3, A, 2 * A, 3 * A)), 90, 'bilinear', 'crop'); c = double(strcmp(class(R), 'uint8')); R2 = double(R(:, :, 2)); R3 = double(R(:, :, 3));",
    );
    assert_eq!(matrix(&e, "c").data(), [1.]);
    let twice: Vec<f64> = ccw.iter().map(|v| 2. * v).collect();
    let thrice: Vec<f64> = ccw.iter().map(|v| 3. * v).collect();
    assert_eq!(matrix(&e, "R2").data(), twice.as_slice());
    assert_eq!(matrix(&e, "R3").data(), thrice.as_slice());
}

#[test]
fn imresize_tiny_scales_return_quickly_or_error() {
    let mut e = Engine::default();
    let start = std::time::Instant::now();
    // With antialiasing the kernel would span 4/scale samples: rejected
    // before anything is allocated.
    for scale in ["1e-300", "1e-9"] {
        for method in ["bicubic", "bilinear"] {
            assert!(
                error(&mut e, &format!("imresize(ones(4), {scale}, '{method}')"))
                    .contains("imresize: the scale is too small to antialias"),
                "{scale} {method}"
            );
        }
        // Without antialiasing (and for 'nearest') the one output pixel
        // samples far past the last row and column: the replicated corner.
        for options in ["'Antialiasing', false", "'nearest'"] {
            assert_values(
                &mut e,
                &format!("imresize(magic(4), {scale}, {options})"),
                &[1, 1],
                &[1.],
                0.,
            );
        }
    }
    // The cap is 2^20 taps for a small image: 4/scale = 2^20 - 2 (+2) fits,
    // 4/scale = 2^20 (+2) does not.
    assert_values(
        &mut e,
        "imresize(ones(4), 4 / (2^20 - 2))",
        &[1, 1],
        &[1.],
        1e-9,
    );
    assert!(
        error(&mut e, "imresize(ones(4), 4 / 2^20)")
            .contains("imresize: the scale is too small to antialias")
    );
    // Ordinary strong shrinks still antialias: 0.1 and [1 1] give the mean
    // of a constant image.
    assert_values(&mut e, "imresize(5 * ones(4), 0.1)", &[1, 1], &[5.], 1e-12);
    assert_values(
        &mut e,
        "imresize(5 * ones(7, 9), [1 1])",
        &[1, 1],
        &[5.],
        1e-12,
    );
    let elapsed = start.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "took {elapsed:?}"
    );
}

#[test]
fn imgaussfilt_default_size_is_bounded() {
    let mut e = Engine::default();
    // 2*ceil(2*sigma)+1 for sigma 1e12 is far over the 4096 limit.
    let message = error(&mut e, "imgaussfilt(ones(4), 1e12)");
    assert!(
        message.contains("imgaussfilt: FilterSize must be odd positive integers up to 4096"),
        "{message}"
    );
    assert!(error(&mut e, "imgaussfilt(ones(4), [1 1e12])").contains("up to 4096"));
    // sigma 1023.6 needs 4097; 1023.5 needs 4095, the largest odd size allowed.
    assert!(error(&mut e, "imgaussfilt(ones(4), 1023.6)").contains("up to 4096"));
    assert_values(
        &mut e,
        "imgaussfilt(ones(4), 1023.5)",
        &[4, 4],
        &[1.; 16],
        1e-12,
    );
    // An explicit FilterSize replaces the default, so a huge sigma works.
    assert_values(
        &mut e,
        "imgaussfilt(ones(4), 1e12, 'FilterSize', 3)",
        &[4, 4],
        &[1.; 16],
        1e-12,
    );
    // The explicit size meets the same check and text.
    assert!(error(&mut e, "imgaussfilt(ones(4), 1, 'FilterSize', 4097)").contains("up to 4096"));
}

#[test]
fn imfilter_and_medfilt2_bound_the_padded_copy_before_allocating() {
    let mut e = Engine::default();
    let start = std::time::Instant::now();
    // The output of imfilter(ones(4096), ones(1, 1e6)) is 4096-by-4096, but
    // the padded copy the kernel reads would be 4096-by-1004095 doubles (33 GB).
    run(&mut e, "A = ones(4096); K = ones(1, 1e6);");
    let message = error(&mut e, "imfilter(A, K)");
    assert!(
        message
            .contains("imfilter: the 1-by-1000000 filter is too large for a 4096-by-4096 result"),
        "{message}"
    );
    // 'conv' and every boundary meet the same check; 'full' is already over
    // the output limit (4096-by-1004095).
    for options in ["'conv', 'circular'", "'symmetric'", "5"] {
        let message = error(&mut e, &format!("imfilter(A, K, {options})"));
        assert!(message.contains("is too large"), "{options}: {message}");
    }
    let message = error(&mut e, "imfilter(A, K, 'full')");
    assert!(
        message.contains("output exceeds the 50331648-element limit"),
        "{message}"
    );
    // imgaussfilt uses the same filter code and names itself.
    run(&mut e, "clear A K; B = ones(1, 12000000);");
    let message = error(&mut e, "imgaussfilt(B, 1)");
    assert!(
        message.contains("imgaussfilt: the 5-by-1 filter is too large"),
        "{message}"
    );
    // A window much larger than the image: 1-by-1e7 with 4096-by-1 is 4e10 doubles.
    run(&mut e, "clear B; Z = zeros(1, 1e7);");
    let message = error(&mut e, "medfilt2(Z, [4096 1])");
    assert!(
        message.contains(
            "medfilt2: the 4096-by-1 neighbourhood is too large for a 1-by-10000000 result"
        ),
        "{message}"
    );
    assert!(
        start.elapsed() < std::time::Duration::from_secs(10),
        "took {:?}",
        start.elapsed()
    );
    // Windows and kernels that fit still work, also larger than the image.
    assert_values(
        &mut e,
        "imfilter(ones(1, 2), ones(1, 9), 'full')",
        &[1, 10],
        &[1., 2., 2., 2., 2., 2., 2., 2., 2., 1.],
        0.,
    );
    assert_values(&mut e, "medfilt2(ones(2), [5 5])", &[2, 2], &[0.; 4], 0.);
}

#[test]
fn medfilt2_ranks_every_nan_after_every_number() {
    let mut e = Engine::default();
    // 0/0 is a NaN with the sign bit set on x86; abs clears it. Both must
    // rank after every number, so the two inputs give the same result.
    run(&mut e, "N = 0/0; P = abs(N);");
    // Zero padding. Odd window 1x3: the middle of {0, 1, NaN} is 1, of
    // {1, NaN, 3} is 3, of {NaN, 3, 0} is 3; the other rows have no NaN.
    let odd = [1., 4., 7., 3., 5., 8., 3., 5., 8.];
    // Even window 2x2, the mean of the two middle values with NaN last:
    // {1,NaN,3,4} -> (3+4)/2; {3,4,0,0} -> (0+3)/2; {NaN,4,0,0} -> (0+4)/2;
    // {4,0,0,0} -> 0.
    let even = [3.5, 1.5, 2., 0.];
    for nan in ["N", "P"] {
        assert_values(
            &mut e,
            &format!("medfilt2([1 {nan} 3; 4 5 6; 7 8 9], [1 3])"),
            &[3, 3],
            &odd,
            0.,
        );
        assert_values(
            &mut e,
            &format!("medfilt2([1 {nan}; 3 4], [2 2])"),
            &[2, 2],
            &even,
            0.,
        );
    }
    // A window of nothing but NaN stays NaN.
    assert_values(
        &mut e,
        "medfilt2([N N N], [1 3], 'symmetric')",
        &[1, 3],
        &[f64::NAN; 3],
        0.,
    );
}

#[test]
fn large_filters_stop_promptly_when_cancelled() {
    use std::sync::atomic::Ordering;
    let mut e = Engine::default();
    // Uncancelled these run for tens of seconds to minutes: imfilter does
    // 1024^2 x 90000 = 9.4e10 multiply-adds, medfilt2 sorts a 151x151 window
    // at 512^2 pixels.
    run(
        &mut e,
        "A = ones(1024); K = ones(300) / 90000; M = ones(512);",
    );
    for code in [
        "imfilter(A, K);",
        "medfilt2(M, [151 151]);",
        "imgaussfilt(A, 700, 'FilterSize', 4095);",
    ] {
        let cancel = e.cancellation();
        let timer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            cancel.store(true, Ordering::Relaxed);
        });
        let start = std::time::Instant::now();
        let message = error(&mut e, code);
        timer.join().unwrap();
        assert!(message.contains("cancelled"), "{code}: {message}");
        assert!(
            start.elapsed() < std::time::Duration::from_secs(15),
            "{code}: took {:?}",
            start.elapsed()
        );
        e.cancellation().store(false, Ordering::Relaxed);
    }
    // The engine still works after a cancelled filter.
    assert_values(&mut e, "imfilter([1 2 3], 2)", &[1, 3], &[2., 4., 6.], 0.);
}
