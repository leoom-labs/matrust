//! Image registration pieces 7c3a and 7c3b: imregtform and imregister with
//! the MeanSquares and MattesMutualInformation metrics and the
//! RegularStepGradientDescent and OnePlusOneEvolutionary optimizers.
//!
//! Expected values: the synthetic fixed image (Gaussian blobs that fade to 0
//! before the border) is warped by a known transformation T0 with imwarp
//! (cubic, piece 7c2), so the true answer is invert(T0). Recovery is checked
//! at the image corners: the largest distance between the recovered and the
//! true transformation of the four world corners must be at most 0.1 pixel
//! (the 7c done-check). The other checks follow the public MathWorks
//! imregtform/imregister reference pages (outputs, OutputView = Rfixed,
//! fill 0, class kept). The multimodal case registers 255 - (the warped
//! image) to the image itself, where only mutual information can find the
//! answer. No MATLAB.
use matrust::{Array, Engine, Matrix};

fn matrix(e: &Engine, name: &str) -> Matrix {
    if let Some(m) = e.variables.get(name) {
        return m.clone();
    }
    match e.array(name) {
        Some(Array::Numeric(m)) => m.clone(),
        other => panic!("missing numeric {name}: {other:?}"),
    }
}

fn run(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(out) => out.output.join("\n"),
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

fn scalar(e: &mut Engine, expr: &str) -> f64 {
    run(e, &format!("zz_s = double({expr});"));
    let m = matrix(e, "zz_s");
    assert_eq!(m.data().len(), 1, "{expr} is not a scalar");
    m.data()[0]
}

fn values(e: &mut Engine, expr: &str) -> Vec<f64> {
    run(e, &format!("zz_v = double({expr}); zz_v = zz_v(:)';"));
    matrix(e, "zz_v").data().to_vec()
}

fn text(e: &mut Engine, expr: &str) -> String {
    run(e, &format!("zz_c = double({expr});"));
    matrix(e, "zz_c")
        .data()
        .iter()
        .map(|&c| char::from_u32(c as u32).unwrap())
        .collect()
}

/// 80-by-96 fixed image of ten Gaussian blobs (value about 0 at the border),
/// the monomodal and multimodal configurations (300 evolutionary iterations
/// per level: with the default 100 the small default InitialRadius does not
/// reach these few-pixel warps), the world corners C and the centre c.
const SETUP: &str = "[x, y] = meshgrid(1:96, 1:80); fixed = zeros(80, 96); \
B = [30 28 6 1.0; 52 24 4 0.9; 66 40 5 0.8; 44 50 7 0.7; 28 58 4 1.0; 70 60 3.5 0.9; 58 52 3 0.6; 40 36 3 0.8; 76 22 3 0.7; 22 40 3.5 0.6]; \
for k = 1:size(B, 1), fixed = fixed + B(k, 4)*exp(-((x - B(k, 1)).^2 + (y - B(k, 2)).^2)/(2*B(k, 3)^2)); end; \
[optimizer, metric] = imregconfig('monomodal'); \
[evolution, mattes] = imregconfig('multimodal'); evolution.MaximumIterations = 300; \
C = [0.5 0.5; 96.5 0.5; 0.5 80.5; 96.5 80.5]; c = [48.5; 40.5]; Rf = imref2d(size(fixed));";

fn engine() -> Engine {
    let mut e = Engine::default();
    run(&mut e, SETUP);
    e
}

/// Largest corner distance (pixels) between `tform` and invert(T0) at the
/// world corners `corners`, divided by `extent` (world units per pixel).
fn corner_error(e: &mut Engine, tform: &str, corners: &str, extent: f64) -> f64 {
    run(
        e,
        &format!(
            "P = transformPointsForward({tform}, {corners}); Q = transformPointsForward(invert(T0), {corners}); zz_err = max(sqrt(sum((P - Q).^2, 2)));"
        ),
    );
    matrix(e, "zz_err").data()[0] / extent
}

/// moving = imwarp(fixed, T0) in the fixed frame; returns the corner error
/// of imregtform(moving, fixed, type, optimizer, metric).
fn recover(e: &mut Engine, t0: &str, ty: &str, class: &str) -> f64 {
    recover_with(e, t0, ty, class, "optimizer", "metric")
}

/// As [`recover`] with the optimizer and metric variables named.
fn recover_with(
    e: &mut Engine,
    t0: &str,
    ty: &str,
    class: &str,
    optimizer: &str,
    metric: &str,
) -> f64 {
    run(
        e,
        &format!(
            "{t0}; moving = imwarp(fixed, T0, 'cubic', 'OutputView', Rf); tform = imregtform(moving, fixed, '{ty}', {optimizer}, {metric});"
        ),
    );
    assert_eq!(text(e, "class(tform)"), class, "{ty}");
    let err = corner_error(e, "tform", "C", 1.);
    eprintln!("{ty} {optimizer} {metric}: corner error {err:.3e} px");
    err
}

/// The synthetic warps of the four transformation types.
const TRANSLATION: &str = "T0 = transltform2d(3.4, -2.3)";
const RIGID: &str =
    "R = [cosd(6) -sind(6); sind(6) cosd(6)]; T0 = rigidtform2d(6, (c - R*c + [1.5; -2.25])')";
const SIMILARITY: &str = "S = 1.05*[cosd(-4) -sind(-4); sind(-4) cosd(-4)]; T0 = simtform2d(1.05, -4, (c - S*c + [-1.2; 0.8])')";
const AFFINE: &str =
    "A2 = [1.04 0.06; -0.03 0.97]; T0 = affinetform2d([A2, c - A2*c + [0.9; 1.3]; 0 0 1])";

/// Mattes + regular step, mean squares + one-plus-one and Mattes +
/// one-plus-one each recover the warp within a tenth of a pixel.
fn recover_with_every_new_pair(t0: &str, ty: &str, class: &str) {
    let mut e = engine();
    for (optimizer, metric) in [
        ("optimizer", "mattes"),
        ("evolution", "metric"),
        ("evolution", "mattes"),
    ] {
        let start = std::time::Instant::now();
        let err = recover_with(&mut e, t0, ty, class, optimizer, metric);
        assert!(
            err <= 0.1,
            "{ty} with {optimizer} and {metric}: corner error {err}"
        );
        let took = start.elapsed().as_secs_f64();
        assert!(
            took < 2.,
            "{ty} with {optimizer} and {metric} took {took} s"
        );
    }
}

#[test]
fn translation_is_recovered_within_a_tenth_of_a_pixel() {
    let mut e = engine();
    let err = recover(&mut e, TRANSLATION, "translation", "transltform2d");
    assert!(err <= 0.1, "translation corner error {err}");
}

#[test]
fn rigid_is_recovered_within_a_tenth_of_a_pixel() {
    let mut e = engine();
    let err = recover(&mut e, RIGID, "rigid", "rigidtform2d");
    assert!(err <= 0.1, "rigid corner error {err}");
    // The angle is the inverse rotation.
    let angle = scalar(&mut e, "tform.RotationAngle");
    assert!((angle + 6.).abs() < 0.05, "RotationAngle {angle}");
}

#[test]
fn similarity_is_recovered_within_a_tenth_of_a_pixel() {
    let mut e = engine();
    let err = recover(&mut e, SIMILARITY, "similarity", "simtform2d");
    assert!(err <= 0.1, "similarity corner error {err}");
    let scale = scalar(&mut e, "tform.Scale");
    assert!((scale - 1. / 1.05).abs() < 1e-3, "Scale {scale}");
}

#[test]
fn affine_is_recovered_within_a_tenth_of_a_pixel() {
    let mut e = engine();
    let err = recover(&mut e, AFFINE, "affine", "affinetform2d");
    assert!(err <= 0.1, "affine corner error {err}");
}

#[test]
fn mattes_and_evolutionary_recover_translation() {
    recover_with_every_new_pair(TRANSLATION, "translation", "transltform2d");
}

#[test]
fn mattes_and_evolutionary_recover_rigid() {
    recover_with_every_new_pair(RIGID, "rigid", "rigidtform2d");
}

#[test]
fn mattes_and_evolutionary_recover_similarity() {
    recover_with_every_new_pair(SIMILARITY, "similarity", "simtform2d");
}

#[test]
fn mattes_and_evolutionary_recover_affine() {
    recover_with_every_new_pair(AFFINE, "affine", "affinetform2d");
}

#[test]
fn mattes_registers_inverted_intensities() {
    // moving = 255 - (the warped image): the brightness is reversed, so
    // mean squares cannot match it but mutual information can.
    let mut e = engine();
    run(
        &mut e,
        &format!(
            "{RIGID}; F = 255*fixed/max(fixed(:)); moving = 255 - imwarp(F, T0, 'cubic', 'OutputView', Rf); \
             t_ms = imregtform(moving, F, 'rigid', optimizer, metric); \
             t_mi = imregtform(moving, F, 'rigid', optimizer, mattes); \
             t_ev = imregtform(moving, F, 'rigid', evolution, mattes); \
             reg = imregister(moving, F, 'rigid', evolution, mattes);"
        ),
    );
    let ms = corner_error(&mut e, "t_ms", "C", 1.);
    assert!(ms > 1., "mean squares should fail here ({ms} px)");
    for t in ["t_mi", "t_ev"] {
        let err = corner_error(&mut e, t, "C", 1.);
        eprintln!("inverted {t}: corner error {err:.3e} px");
        assert!(err <= 0.1, "{t}: corner error {err}");
    }
    // The registered image is the inverted image moved onto F's grid.
    let diff = scalar(
        &mut e,
        "max(max(abs(255 - reg(10:70, 10:86) - F(10:70, 10:86))))",
    );
    assert!(diff < 5., "registered differs from 255 - F by {diff}");
}

#[test]
fn registrations_repeat_exactly() {
    let mut e = engine();
    run(
        &mut e,
        &format!(
            "{SIMILARITY}; moving = imwarp(fixed, T0, 'cubic', 'OutputView', Rf); \
             few = mattes; few.UseAllPixels = false; few.NumberOfSpatialSamples = 2000; \
             a1 = imregtform(moving, fixed, 'similarity', evolution, mattes); \
             a2 = imregtform(moving, fixed, 'similarity', evolution, mattes); \
             b1 = imregtform(moving, fixed, 'similarity', evolution, metric); \
             b2 = imregtform(moving, fixed, 'similarity', evolution, metric); \
             c1 = imregtform(moving, fixed, 'similarity', optimizer, few); \
             c2 = imregtform(moving, fixed, 'similarity', optimizer, few); \
             d1 = imregtform(moving, fixed, 'similarity', evolution, few); \
             d2 = imregtform(moving, fixed, 'similarity', evolution, few); \
             call = imregtform(moving, fixed, 'similarity', optimizer, mattes);"
        ),
    );
    for (one, two) in [("a1", "a2"), ("b1", "b2"), ("c1", "c2"), ("d1", "d2")] {
        assert_eq!(
            scalar(&mut e, &format!("isequal({one}.A, {two}.A)")),
            1.,
            "{one} and {two} differ"
        );
    }
    // Sampled Mattes: another answer than every pixel gives, still close.
    assert_eq!(scalar(&mut e, "isequal(c1.A, call.A)"), 0.);
    for t in ["c1", "d1"] {
        let err = corner_error(&mut e, t, "C", 1.);
        eprintln!("2000 samples {t}: corner error {err:.3e} px");
        assert!(err <= 0.1, "{t}: corner error {err}");
    }
}

#[test]
fn mattes_histogram_bins_and_samples_are_used() {
    let mut e = engine();
    run(
        &mut e,
        &format!(
            "{RIGID}; moving = imwarp(fixed, T0, 'cubic', 'OutputView', Rf); \
             m32 = mattes; m32.NumberOfHistogramBins = 32; \
             t32 = imregtform(moving, fixed, 'rigid', optimizer, m32); \
             t50 = imregtform(moving, fixed, 'rigid', optimizer, mattes); \
             tiny = mattes; tiny.UseAllPixels = false; tiny.NumberOfSpatialSamples = 1; \
             many = mattes; many.UseAllPixels = false; many.NumberOfSpatialSamples = 1e9; \
             tmany = imregtform(moving, fixed, 'rigid', optimizer, many); \
             big = mattes; big.NumberOfHistogramBins = 5000;"
        ),
    );
    assert_eq!(scalar(&mut e, "isequal(t32.A, t50.A)"), 0.);
    let err = corner_error(&mut e, "t32", "C", 1.);
    assert!(err <= 0.1, "32 bins: corner error {err}");
    // More samples than pixels: every pixel, as UseAllPixels.
    assert_eq!(scalar(&mut e, "isequal(tmany.A, t50.A)"), 1.);
    // One sample per level runs (the answer means little).
    run(
        &mut e,
        "t1 = imregtform(moving, fixed, 'rigid', optimizer, tiny);",
    );
    assert_eq!(text(&mut e, "class(t1)"), "rigidtform2d");
    error_has(
        &mut e,
        "imregtform(moving, fixed, 'rigid', optimizer, big)",
        "imregtform: NumberOfHistogramBins of the MattesMutualInformation metric must be an integer from 5 to 4096",
    );
}

#[test]
fn imregister_is_imregtform_then_imwarp_onto_the_fixed_grid() {
    let mut e = engine();
    run(
        &mut e,
        "R = [cosd(5) -sind(5); sind(5) cosd(5)]; T0 = rigidtform2d(5, (c - R*c + [1; 1.5])'); \
         moving = imwarp(fixed, T0, 'cubic', 'OutputView', Rf); \
         [reg, Rreg] = imregister(moving, fixed, 'rigid', optimizer, metric); \
         tform = imregtform(moving, fixed, 'rigid', optimizer, metric); \
         same = isequal(reg, imwarp(moving, tform, 'OutputView', Rf)); \
         one = imregister(moving, fixed, 'rigid', optimizer, metric);",
    );
    assert_eq!(scalar(&mut e, "same"), 1.);
    assert_eq!(scalar(&mut e, "isequal(one, reg)"), 1.);
    assert_eq!(values(&mut e, "size(reg)"), [80., 96.]);
    assert_eq!(text(&mut e, "class(reg)"), "double");
    assert_eq!(text(&mut e, "class(Rreg)"), "imref2d");
    assert_eq!(values(&mut e, "Rreg.ImageSize"), [80., 96.]);
    assert_eq!(values(&mut e, "Rreg.XWorldLimits"), [0.5, 96.5]);
    assert_eq!(values(&mut e, "Rreg.YWorldLimits"), [0.5, 80.5]);
    // Registered close to fixed away from the fill (0) at the borders.
    let diff = scalar(
        &mut e,
        "max(max(abs(reg(10:70, 10:86) - fixed(10:70, 10:86))))",
    );
    assert!(diff < 0.02, "registered differs from fixed by {diff}");
    // Fill pixels are 0: the top-left corner maps outside the moving image.
    run(
        &mut e,
        "big = imregister(moving + 1, fixed + 1, 'rigid', optimizer, metric);",
    );
    assert_eq!(scalar(&mut e, "big(1, 1)"), 0.);
}

#[test]
fn world_limits_of_rmoving_and_rfixed_are_used() {
    let mut e = engine();
    // Fixed placed at x in [10 58], y in [-5 35] (pixel extent 0.5); the
    // moving image is imwarp's default view, larger and offset.
    run(
        &mut e,
        "Rw = imref2d(size(fixed), [10 58], [-5 35]); cw = [34; 15]; \
         R = [cosd(4) -sind(4); sind(4) cosd(4)]; T0 = rigidtform2d(4, (cw - R*cw + [0.6; -0.4])'); \
         [moving, Rm] = imwarp(fixed, Rw, T0, 'cubic'); \
         tform = imregtform(moving, Rm, fixed, Rw, 'rigid', optimizer, metric); \
         Cw = [10 -5; 58 -5; 10 35; 58 35]; \
         [reg, Rreg] = imregister(moving, Rm, fixed, Rw, 'rigid', optimizer, metric); \
         same = isequal(reg, imwarp(moving, Rm, tform, 'OutputView', Rw));",
    );
    assert_ne!(values(&mut e, "size(moving)"), [80., 96.]);
    let err = corner_error(&mut e, "tform", "Cw", 0.5);
    assert!(err <= 0.1, "referenced rigid corner error {err} px");
    assert_eq!(scalar(&mut e, "same"), 1.);
    assert_eq!(values(&mut e, "Rreg.XWorldLimits"), [10., 58.]);
    assert_eq!(values(&mut e, "Rreg.YWorldLimits"), [-5., 35.]);
    assert_eq!(values(&mut e, "size(reg)"), [80., 96.]);
    // Without the referencing objects the same images give another answer.
    run(
        &mut e,
        "plain = imregtform(moving, fixed, 'rigid', optimizer, metric);",
    );
    let err = corner_error(&mut e, "plain", "Cw", 0.5);
    assert!(err > 1., "unreferenced call should not match ({err} px)");
}

#[test]
fn initial_transformation_is_the_starting_point() {
    let mut e = engine();
    // A step below MinimumStepLength stops before the first step: the
    // result is the starting point itself.
    run(
        &mut e,
        "T0 = transltform2d(3.4, -2.3); moving = imwarp(fixed, T0, 'cubic', 'OutputView', Rf); \
         o = optimizer; o.MaximumStepLength = 1e-6; \
         t1 = imregtform(moving, fixed, 'translation', o, metric, 'InitialTransformation', transltform2d(-3, 2)); \
         t0 = imregtform(moving, fixed, 'translation', o, metric); \
         r1 = imregtform(moving, fixed, 'rigid', o, metric, 'InitialTransformation', rigidtform2d(10, [1 2]));",
    );
    assert_eq!(values(&mut e, "t1.Translation"), [-3., 2.]);
    assert_eq!(values(&mut e, "t0.Translation"), [0., 0.]);
    assert!((scalar(&mut e, "r1.RotationAngle") - 10.).abs() < 1e-12);
    let t = values(&mut e, "r1.Translation");
    assert!(
        (t[0] - 1.).abs() < 1e-12 && (t[1] - 2.).abs() < 1e-12,
        "{t:?}"
    );
    // From a start next to the answer the default settings finish the job;
    // a start of another class is accepted when it has the right form.
    run(
        &mut e,
        "t2 = imregtform(moving, fixed, 'translation', optimizer, metric, 'InitialTransformation', affinetform2d([1 0 -3; 0 1 2; 0 0 1]));",
    );
    assert_eq!(text(&mut e, "class(t2)"), "transltform2d");
    let err = corner_error(&mut e, "t2", "C", 1.);
    assert!(err <= 0.1, "corner error {err}");
    error_has(
        &mut e,
        "imregtform(moving, fixed, 'translation', optimizer, metric, 'InitialTransformation', rigidtform2d(10, [0 0]))",
        "InitialTransformation must be a transltform2d transformation",
    );
    error_has(
        &mut e,
        "imregtform(moving, fixed, 'rigid', optimizer, metric, 'InitialTransformation', eye(3))",
        "InitialTransformation must be a 2-D geometric transformation object",
    );
}

#[test]
fn pyramid_levels_need_large_enough_images() {
    let mut e = engine();
    run(
        &mut e,
        "T0 = transltform2d(1.4, -0.7); moving = imwarp(fixed, T0, 'cubic', 'OutputView', Rf);",
    );
    let message = error(
        &mut e,
        "imregtform(moving, fixed, 'translation', optimizer, metric, 'PyramidLevels', 6)",
    );
    assert!(
        message.contains("imregtform: the images are too small for PyramidLevels = 6")
            && message.contains("at least 128 pixels")
            && message.contains("fixed is 80-by-96"),
        "{message}"
    );
    error_has(
        &mut e,
        "imregister(moving(1:15, :), fixed, 'translation', optimizer, metric)",
        "imregister: the images are too small for PyramidLevels = 3",
    );
    for bad in ["0", "2.5", "-1", "[2 3]", "'three'"] {
        error_has(
            &mut e,
            &format!(
                "imregtform(moving, fixed, 'translation', optimizer, metric, 'PyramidLevels', {bad})"
            ),
            "PyramidLevels must be a positive integer",
        );
    }
    // One level and five levels (64 pixels needed) both work.
    for levels in [1, 5] {
        run(
            &mut e,
            &format!(
                "tform = imregtform(moving, fixed, 'translation', optimizer, metric, 'PyramidLevels', {levels});"
            ),
        );
        let err = corner_error(&mut e, "tform", "C", 1.);
        assert!(err <= 0.1, "PyramidLevels {levels}: corner error {err}");
    }
}

#[test]
fn optimizer_and_metric_arguments_are_checked() {
    let mut e = engine();
    run(
        &mut e,
        "moving = fixed; [o2, m2] = imregconfig('multimodal');",
    );
    error_has(
        &mut e,
        "imregtform(moving, fixed, 'rigid', metric, optimizer)",
        "imregtform: OPTIMIZER must be a registration.optimizer object (RegularStepGradientDescent or OnePlusOneEvolutionary, made by imregconfig), not a registration.metric.MeanSquares (the optimizer comes before the metric)",
    );
    error_has(
        &mut e,
        "imregister(moving, fixed, 'rigid', optimizer, 3)",
        "imregister: METRIC must be a registration.metric object (MeanSquares or MattesMutualInformation, made by imregconfig), not a double",
    );
    error_has(
        &mut e,
        "imregtform(moving, fixed, 'rigid', 'RegularStepGradientDescent', metric)",
        "not a char",
    );
    // The multimodal pair is accepted (images equal: hardly any change;
    // mutual information with interpolation is not exactly flat there).
    run(
        &mut e,
        "t2 = imregtform(moving, fixed, 'rigid', o2, m2, 'PyramidLevels', 1);",
    );
    let change = scalar(&mut e, "max(abs(t2.A(:) - [1; 0; 0; 0; 1; 0; 0; 0; 1]))");
    assert!(change < 0.02, "{change}");
    error_has(
        &mut e,
        "imregtform(moving, fixed, 'projective', optimizer, metric)",
        "transformType must be 'translation', 'rigid', 'similarity' or 'affine', not 'projective'",
    );
    error_has(
        &mut e,
        "imregtform(moving, fixed, 'rigid', optimizer)",
        "imregtform: not enough input arguments",
    );
    error_has(
        &mut e,
        "imregtform(moving, fixed, 'rigid', optimizer, metric, 'Display')",
        "the parameter 'DisplayOptimization' needs a value",
    );
    error_has(
        &mut e,
        "imregtform(moving, fixed, 'rigid', optimizer, metric, 'Interp', 'cubic')",
        "unknown parameter 'Interp'",
    );
    error_has(
        &mut e,
        "[a, b] = imregtform(moving, fixed, 'rigid', optimizer, metric)",
        "Too many output arguments",
    );
    error_has(
        &mut e,
        "[a, b, c] = imregister(moving, fixed, 'rigid', optimizer, metric)",
        "Too many output arguments",
    );
    error_has(
        &mut e,
        "imregtform(cat(3, moving, moving, moving), fixed, 'rigid', optimizer, metric)",
        "moving must be a 2-D grayscale image",
    );
    error_has(
        &mut e,
        "imregtform(moving > 0.5, fixed, 'rigid', optimizer, metric)",
        "moving must be single, double, int8, int16, int32, uint8, uint16 or uint32, not logical",
    );
    error_has(
        &mut e,
        "imregtform(moving, imref2d([10 10]), fixed, Rf, 'rigid', optimizer, metric)",
        "Rmoving.ImageSize [10 10] must match the size of moving (80-by-96)",
    );
    error_has(
        &mut e,
        "imregtform(moving, Rf, fixed, 7, 'rigid', optimizer, metric)",
        "Rfixed must be an imref2d object",
    );
    // Unique prefixes of the type and the parameter names, any case.
    run(
        &mut e,
        "t = imregtform(moving, fixed, 'TRANS', optimizer, metric, 'pyramid', 2, 'displayopt', false);",
    );
    assert_eq!(text(&mut e, "class(t)"), "transltform2d");
}

#[test]
fn uint8_images_register_and_keep_their_class() {
    let mut e = engine();
    run(
        &mut e,
        "f8 = im2uint8(fixed / max(fixed(:))); \
         R = [cosd(-5) -sind(-5); sind(-5) cosd(-5)]; T0 = rigidtform2d(-5, (c - R*c + [-1.75; 1.25])'); \
         m8 = imwarp(f8, T0, 'cubic', 'OutputView', Rf); \
         [reg, Rreg] = imregister(m8, f8, 'rigid', optimizer, metric); \
         tform = imregtform(m8, f8, 'rigid', optimizer, metric); \
         same = isequal(reg, imwarp(m8, tform, 'OutputView', Rf));",
    );
    assert_eq!(text(&mut e, "class(reg)"), "uint8");
    assert_eq!(scalar(&mut e, "same"), 1.);
    let err = corner_error(&mut e, "tform", "C", 1.);
    assert!(err <= 0.1, "uint8 rigid corner error {err}");
    // single works too and stays single.
    run(
        &mut e,
        "rs = imregister(single(m8), single(f8), 'translation', optimizer, metric);",
    );
    assert_eq!(text(&mut e, "class(rs)"), "single");
}

#[test]
fn display_optimization_prints_each_iteration() {
    let mut e = engine();
    run(
        &mut e,
        "T0 = transltform2d(1.2, 0.8); moving = imwarp(fixed, T0, 'cubic', 'OutputView', Rf);",
    );
    let quiet = run(
        &mut e,
        "t = imregtform(moving, fixed, 'translation', optimizer, metric);",
    );
    assert!(quiet.is_empty(), "{quiet}");
    let shown = run(
        &mut e,
        "t = imregtform(moving, fixed, 'translation', optimizer, metric, 'DisplayOptimization', true);",
    );
    for part in [
        "Pyramid level 1 of 3 (20-by-24 fixed image):",
        "Pyramid level 2 of 3 (40-by-48 fixed image):",
        "Pyramid level 3 of 3 (80-by-96 fixed image):",
        "  iteration    1  metric ",
        "  stopped after ",
    ] {
        assert!(shown.contains(part), "missing {part:?} in\n{shown}");
    }
    let iterations = shown
        .lines()
        .filter(|l| l.starts_with("  iteration"))
        .count();
    assert!(iterations >= 6, "{iterations} iteration lines\n{shown}");
    assert_eq!(
        shown.lines().filter(|l| l.starts_with("  stopped")).count(),
        3
    );
}

#[test]
fn a_cancellation_stops_a_long_registration() {
    use std::sync::atomic::Ordering;
    let mut e = Engine::default();
    run(
        &mut e,
        "[x, y] = meshgrid(1:700); F = sin(x/9) .* cos(y/13) + exp(-((x - 300).^2 + (y - 420).^2)/5000); \
         M = imwarp(F, transltform2d(2, -1), 'OutputView', imref2d(size(F))); \
         [o, m] = imregconfig('monomodal'); o.MaximumIterations = 1000; o.MinimumStepLength = 1e-12; o.GradientMagnitudeTolerance = 1e-30;",
    );
    let code = "t = imregtform(M, F, 'affine', o, m, 'PyramidLevels', 1);";
    let stop = e.cancellation();
    let watcher = std::thread::spawn({
        let stop = stop.clone();
        move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            stop.store(true, Ordering::Relaxed);
        }
    });
    let start = std::time::Instant::now();
    let message = error(&mut e, code);
    watcher.join().unwrap();
    stop.store(false, Ordering::Relaxed);
    assert!(message.contains("cancelled"), "{message}");
    assert!(start.elapsed().as_secs_f64() < 3., "{:?}", start.elapsed());
    run(
        &mut e,
        "t = imregtform(M(1:40, 1:40), F(1:40, 1:40), 'translation', o, m, 'PyramidLevels', 1);",
    );
}

#[test]
fn display_optimization_shows_the_evolutionary_radius() {
    let mut e = engine();
    run(
        &mut e,
        "T0 = transltform2d(1.2, 0.8); moving = imwarp(fixed, T0, 'cubic', 'OutputView', Rf); \
         o = evolution; o.MaximumIterations = 7;",
    );
    let shown = run(
        &mut e,
        "t = imregtform(moving, fixed, 'translation', o, mattes, 'DisplayOptimization', true, 'PyramidLevels', 2);",
    );
    for part in [
        "Pyramid level 1 of 2 (40-by-48 fixed image):",
        "Pyramid level 2 of 2 (80-by-96 fixed image):",
        "  iteration    1  metric -",
        "  radius 8.8388e-3",
        "  stopped after 7 iterations: MaximumIterations reached",
    ] {
        assert!(shown.contains(part), "missing {part:?} in\n{shown}");
    }
    let iterations = shown
        .lines()
        .filter(|l| l.starts_with("  iteration"))
        .count();
    assert_eq!(iterations, 14, "{shown}");
    // A large Epsilon stops at the first check.
    run(&mut e, "o.Epsilon = 1;");
    let shown = run(
        &mut e,
        "t = imregtform(moving, fixed, 'translation', o, mattes, 'DisplayOptimization', true, 'PyramidLevels', 1);",
    );
    assert!(
        shown.contains("  stopped after 1 iterations: search radius at most Epsilon"),
        "{shown}"
    );
}

#[test]
fn bare_class_names_construct_default_objects() {
    let mut e = Engine::default();
    run(
        &mut e,
        "t = rigidtform2d; a = affinetform2d; s = simtform2d; tr = transltform2d; old = affine2d; R = imref2d;",
    );
    for (name, class) in [
        ("t", "rigidtform2d"),
        ("a", "affinetform2d"),
        ("s", "simtform2d"),
        ("tr", "transltform2d"),
        ("old", "affine2d"),
        ("R", "imref2d"),
    ] {
        assert_eq!(text(&mut e, &format!("class({name})")), class);
    }
    assert_eq!(values(&mut e, "t.A"), [1., 0., 0., 0., 1., 0., 0., 0., 1.]);
    assert_eq!(values(&mut e, "old.T"), values(&mut e, "affine2d().T"));
    assert_eq!(values(&mut e, "R.ImageSize"), [2., 2.]);
    assert_eq!(scalar(&mut e, "isequal(s.A, simtform2d().A)"), 1.);
    // As a statement and inside expressions.
    let shown = run(&mut e, "rigidtform2d");
    assert!(shown.contains("rigidtform2d with properties"), "{shown}");
    assert_eq!(
        values(&mut e, "transformPointsForward(transltform2d, [3 4])"),
        [3., 4.]
    );
    // A variable of that name still wins.
    run(&mut e, "imref2d = 5; v = imref2d + 1;");
    assert_eq!(scalar(&mut e, "v"), 6.);
}
