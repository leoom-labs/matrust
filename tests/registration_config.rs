//! imregconfig (registration optimizer and metric objects), imfuse and
//! imshowpair.
//!
//! Expected values: the defaults, validation rules and display layout of the
//! public MathWorks reference pages of imregconfig,
//! registration.optimizer.RegularStepGradientDescent,
//! registration.optimizer.OnePlusOneEvolutionary,
//! registration.metric.MeanSquares and
//! registration.metric.MattesMutualInformation. imfuse values are worked by
//! hand from the rules on the imfuse help page (unit scale as im2double, pad
//! with zeros at the bottom/right, scale, composite, im2uint8 = round half
//! away from zero of 255 * clip(x, 0, 1)); each derivation is in a comment.
//! No MATLAB evidence.
use matrust::{Array, Engine, Matrix, engine::Limits};

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
        Err(error) => error.to_string(),
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

fn scalar(e: &mut Engine, expr: &str) -> f64 {
    let (dims, v) = values(e, expr);
    assert_eq!(dims, [1, 1], "{expr} is not a scalar");
    v[0]
}

fn text(e: &mut Engine, expr: &str) -> String {
    values(e, &format!("char({expr})"))
        .1
        .into_iter()
        .map(|c| char::from_u32(c as u32).unwrap())
        .collect()
}

fn truth(e: &mut Engine, expr: &str) -> bool {
    scalar(e, &format!("logical({expr})")) != 0.
}

// ------------------------------------------------------------------ imregconfig

#[test]
fn imregconfig_returns_the_documented_classes() {
    let mut e = Engine::default();
    run(&mut e, "[o, m] = imregconfig('monomodal');");
    assert_eq!(
        text(&mut e, "class(o)"),
        "registration.optimizer.RegularStepGradientDescent"
    );
    assert_eq!(text(&mut e, "class(m)"), "registration.metric.MeanSquares");
    run(&mut e, "[o, m] = imregconfig('multimodal');");
    assert_eq!(
        text(&mut e, "class(o)"),
        "registration.optimizer.OnePlusOneEvolutionary"
    );
    assert_eq!(
        text(&mut e, "class(m)"),
        "registration.metric.MattesMutualInformation"
    );
    // Case and a unique prefix are accepted; one output is the optimizer.
    run(&mut e, "o = imregconfig('MONO');");
    assert_eq!(
        text(&mut e, "class(o)"),
        "registration.optimizer.RegularStepGradientDescent"
    );
    run(&mut e, "imregconfig(\"Multimodal\");");
    assert_eq!(
        text(&mut e, "class(ans)"),
        "registration.optimizer.OnePlusOneEvolutionary"
    );
}

#[test]
fn imregconfig_rejects_bad_modalities_and_arity() {
    let mut e = Engine::default();
    assert!(error(&mut e, "imregconfig('bimodal')").contains("'monomodal' or 'multimodal'"));
    assert!(error(&mut e, "imregconfig('m')").contains("'monomodal' or 'multimodal'"));
    assert!(error(&mut e, "imregconfig(5)").contains("'monomodal' or 'multimodal'"));
    assert!(error(&mut e, "imregconfig()").contains("expected one input"));
    assert!(error(&mut e, "imregconfig('monomodal', 1)").contains("expected one input"));
    assert!(error(&mut e, "[a, b, c] = imregconfig('monomodal')").contains("too many output"));
}

#[test]
fn every_default_value_matches_the_reference_pages() {
    let mut e = Engine::default();
    run(&mut e, "[o, m] = imregconfig('monomodal');");
    assert_eq!(scalar(&mut e, "o.GradientMagnitudeTolerance"), 1e-4);
    assert_eq!(scalar(&mut e, "o.MinimumStepLength"), 1e-5);
    assert_eq!(scalar(&mut e, "o.MaximumStepLength"), 0.0625);
    assert_eq!(scalar(&mut e, "o.MaximumIterations"), 100.);
    assert_eq!(scalar(&mut e, "o.RelaxationFactor"), 0.5);
    assert_eq!(text(&mut e, "class(o.MaximumIterations)"), "double");
    run(&mut e, "[o, m] = imregconfig('multimodal');");
    assert_eq!(scalar(&mut e, "o.GrowthFactor"), 1.05);
    assert_eq!(scalar(&mut e, "o.Epsilon"), 1.5e-6);
    assert_eq!(scalar(&mut e, "o.InitialRadius"), 6.25e-3);
    assert_eq!(scalar(&mut e, "o.MaximumIterations"), 100.);
    assert_eq!(scalar(&mut e, "m.NumberOfSpatialSamples"), 500.);
    assert_eq!(scalar(&mut e, "m.NumberOfHistogramBins"), 50.);
    assert_eq!(scalar(&mut e, "m.UseAllPixels"), 1.);
    assert_eq!(text(&mut e, "class(m.UseAllPixels)"), "logical");
}

#[test]
fn properties_isa_isprop_and_isequal() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[o, m] = imregconfig('monomodal'); [o2, m2] = imregconfig('multimodal');",
    );
    run(
        &mut e,
        "p = properties(o); f = fieldnames(o2); q = properties(m2);",
    );
    assert_eq!(scalar(&mut e, "numel(p)"), 5.);
    assert_eq!(text(&mut e, "p{1}"), "GradientMagnitudeTolerance");
    assert_eq!(text(&mut e, "p{5}"), "RelaxationFactor");
    assert_eq!(
        text(&mut e, "strjoin(f', ',')"),
        "GrowthFactor,Epsilon,InitialRadius,MaximumIterations"
    );
    assert_eq!(
        text(&mut e, "strjoin(q', ',')"),
        "NumberOfSpatialSamples,NumberOfHistogramBins,UseAllPixels"
    );
    assert_eq!(scalar(&mut e, "numel(properties(m))"), 0.);
    assert!(truth(
        &mut e,
        "isa(o, 'registration.optimizer.RegularStepGradientDescent')"
    ));
    assert!(!truth(
        &mut e,
        "isa(o, 'registration.optimizer.OnePlusOneEvolutionary')"
    ));
    assert!(!truth(&mut e, "isa(o, 'handle')"));
    assert!(truth(
        &mut e,
        "isa(m2, 'registration.metric.MattesMutualInformation')"
    ));
    assert!(truth(&mut e, "isobject(m)"));
    assert!(!truth(&mut e, "isstruct(m)"));
    // Values compare by content.
    assert!(truth(&mut e, "isequal(o, imregconfig('monomodal'))"));
    assert!(!truth(&mut e, "isequal(o, o2)"));
    assert!(!truth(&mut e, "isequal(o, 5)"));
    run(&mut e, "o3 = o; o3.MaximumIterations = 101;");
    assert!(!truth(&mut e, "isequal(o, o3)"));
}

#[test]
fn property_assignment_changes_only_the_copy() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[o, m] = imregconfig('monomodal'); o2 = o; o2.MaximumIterations = 300;",
    );
    assert_eq!(scalar(&mut e, "o.MaximumIterations"), 100.);
    assert_eq!(scalar(&mut e, "o2.MaximumIterations"), 300.);
    run(&mut e, "o.MaximumIterations = 250;");
    assert_eq!(scalar(&mut e, "o.MaximumIterations"), 250.);
    assert_eq!(scalar(&mut e, "o2.MaximumIterations"), 300.);
    // Dynamic names, integer-class values (stored as double) and logical flags.
    run(
        &mut e,
        "o.('RelaxationFactor') = 0.25; o.MaximumStepLength = single(0.5);",
    );
    assert_eq!(scalar(&mut e, "o.RelaxationFactor"), 0.25);
    assert_eq!(scalar(&mut e, "o.MaximumStepLength"), 0.5);
    assert_eq!(text(&mut e, "class(o.MaximumStepLength)"), "double");
    run(
        &mut e,
        "[~, mm] = imregconfig('multimodal'); mm.UseAllPixels = false; mm.NumberOfHistogramBins = 5; mm.NumberOfSpatialSamples = uint16(2000);",
    );
    assert_eq!(scalar(&mut e, "mm.UseAllPixels"), 0.);
    assert_eq!(scalar(&mut e, "mm.NumberOfHistogramBins"), 5.);
    assert_eq!(scalar(&mut e, "mm.NumberOfSpatialSamples"), 2000.);
    run(&mut e, "mm.UseAllPixels = 1;");
    assert_eq!(text(&mut e, "class(mm.UseAllPixels)"), "logical");
    // A copy inside a struct or a function argument is independent too.
    run(&mut e, "s.opt = o; s.opt.MaximumIterations = 7;");
    assert_eq!(scalar(&mut e, "s.opt.MaximumIterations"), 7.);
    assert_eq!(scalar(&mut e, "o.MaximumIterations"), 250.);
    // Indexing a property value.
    assert_eq!(scalar(&mut e, "o.MaximumIterations(1)"), 250.);
}

#[test]
fn property_assignment_validates_like_the_reference_pages() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[o, m] = imregconfig('monomodal'); [p, q] = imregconfig('multimodal');",
    );
    let class = "registration.optimizer.RegularStepGradientDescent";
    for (code, message) in [
        (
            "o.MaximumIterations = 0;",
            "MaximumIterations must be a positive integer scalar",
        ),
        (
            "o.MaximumIterations = 2.5;",
            "MaximumIterations must be a positive integer scalar",
        ),
        (
            "o.MaximumIterations = [1 2];",
            "MaximumIterations must be a positive integer scalar",
        ),
        (
            "o.MaximumIterations = Inf;",
            "MaximumIterations must be a positive integer scalar",
        ),
        (
            "o.MaximumIterations = 'a';",
            "MaximumIterations must be a positive integer scalar",
        ),
        (
            "o.GradientMagnitudeTolerance = -1;",
            "must be a positive finite numeric scalar",
        ),
        (
            "o.MinimumStepLength = 0;",
            "must be a positive finite numeric scalar",
        ),
        (
            "o.MaximumStepLength = NaN;",
            "must be a positive finite numeric scalar",
        ),
        ("o.RelaxationFactor = 1;", "between 0 and 1"),
        ("o.RelaxationFactor = 0;", "between 0 and 1"),
        ("o.MaximumIterations = true;", "positive integer scalar"),
    ] {
        let message_seen = error(&mut e, code);
        assert!(message_seen.contains(message), "{code}: {message_seen}");
        assert!(message_seen.contains(class), "{code}: {message_seen}");
    }
    assert!(error(&mut e, "o.Foo = 1;").contains(
        "Unrecognized property 'Foo' for class 'registration.optimizer.RegularStepGradientDescent'"
    ));
    assert!(error(&mut e, "x = o.Foo;").contains("Unrecognized method, property, or field 'Foo'"));
    // Property names are case-sensitive in dot assignment.
    assert!(error(&mut e, "o.maximumiterations = 3;").contains("Unrecognized property"));
    assert!(error(&mut e, "m.Anything = 1;").contains("registration.metric.MeanSquares"));
    assert!(error(&mut e, "p.GrowthFactor = 0;").contains("GrowthFactor must be a positive"));
    assert!(error(&mut e, "q.NumberOfHistogramBins = 4;").contains("at least 5"));
    assert!(error(&mut e, "q.NumberOfSpatialSamples = 0;").contains("positive integer"));
    assert!(error(&mut e, "q.UseAllPixels = 2;").contains("logical scalar"));
    assert!(error(&mut e, "o.MaximumIterations(2) = 3;").contains("complete property value"));
    // A failed assignment leaves the value unchanged.
    assert_eq!(scalar(&mut e, "o.MaximumIterations"), 100.);
}

#[test]
fn constructors_by_full_name_take_name_value_pairs() {
    let mut e = Engine::default();
    run(
        &mut e,
        "a = registration.metric.MeanSquares; b = registration.metric.MeanSquares();",
    );
    assert_eq!(text(&mut e, "class(a)"), "registration.metric.MeanSquares");
    assert!(truth(&mut e, "isequal(a, b)"));
    run(
        &mut e,
        "o = registration.optimizer.OnePlusOneEvolutionary('MaximumIterations', 50, 'growthfactor', 2);",
    );
    assert_eq!(scalar(&mut e, "o.MaximumIterations"), 50.);
    assert_eq!(scalar(&mut e, "o.GrowthFactor"), 2.);
    assert_eq!(scalar(&mut e, "o.Epsilon"), 1.5e-6);
    run(
        &mut e,
        "g = registration.optimizer.RegularStepGradientDescent(MaximumStepLength=0.01);",
    );
    assert_eq!(scalar(&mut e, "g.MaximumStepLength"), 0.01);
    run(
        &mut e,
        "mi = registration.metric.MattesMutualInformation('UseAllPixels', false);",
    );
    assert_eq!(scalar(&mut e, "mi.UseAllPixels"), 0.);
    assert!(
        error(
            &mut e,
            "registration.optimizer.OnePlusOneEvolutionary('Epsilon')"
        )
        .contains("pairs")
    );
    assert!(
        error(
            &mut e,
            "registration.optimizer.OnePlusOneEvolutionary('Bogus', 1)"
        )
        .contains("Unrecognized property 'Bogus'")
    );
    assert!(
        error(
            &mut e,
            "registration.metric.MattesMutualInformation('NumberOfHistogramBins', 2)"
        )
        .contains("at least 5")
    );
    // A variable named registration takes precedence over the package.
    run(
        &mut e,
        "registration.metric.MeanSquares = 3; v = registration.metric.MeanSquares;",
    );
    assert_eq!(scalar(&mut e, "v"), 3.);
}

#[test]
fn display_follows_the_reference_layout() {
    let mut e = Engine::default();
    run(&mut e, "[o, m] = imregconfig('monomodal');");
    let shown = run(&mut e, "o");
    assert!(
        shown.contains(
            "  registration.optimizer.RegularStepGradientDescent\n\n  Properties:\n    GradientMagnitudeTolerance: 1.000000e-04\n             MinimumStepLength: 1.000000e-05\n             MaximumStepLength: 6.250000e-02\n             MaximumIterations: 100\n              RelaxationFactor: 5.000000e-01"
        ),
        "{shown}"
    );
    assert!(shown.contains("o = "), "{shown}");
    let shown = run(&mut e, "m");
    assert!(
        shown.contains("  registration.metric.MeanSquares\n\n  This class has no properties."),
        "{shown}"
    );
    run(&mut e, "[o, m] = imregconfig('multimodal');");
    let shown = run(&mut e, "disp(o)");
    assert!(
        shown.contains(
            "  Properties:\n         GrowthFactor: 1.050000e+00\n              Epsilon: 1.500000e-06\n        InitialRadius: 6.250000e-03\n    MaximumIterations: 100"
        ),
        "{shown}"
    );
    let shown = run(&mut e, "m.NumberOfSpatialSamples = 1234");
    assert!(
        shown.contains(
            "    NumberOfSpatialSamples: 1234\n     NumberOfHistogramBins: 50\n              UseAllPixels: 1"
        ),
        "{shown}"
    );
    // A one-column cell shows each class name whole.
    let shown = run(&mut e, "c = {o; m}");
    assert!(
        shown.contains("[1x1 registration.optimizer.OnePlusOneEvolutionary]")
            && shown.contains("[1x1 registration.metric.MattesMutualInformation]"),
        "{shown}"
    );
}

// ----------------------------------------------------------------------- imfuse

#[test]
fn falsecolor_default_scales_independently_into_green_magenta() {
    let mut e = Engine::default();
    run(
        &mut e,
        "A = uint8([0 100; 200 255]); B = uint8([50 50; 150 250]);",
    );
    // A on the unit scale spans 0..1 already: A8 = A.
    // B spans 50..250: (B - 50) / 200 = [0 0; 0.5 1] -> [0 0; 128 255]
    // (127.5 rounds half away from zero). green-magenta = [2 1 2]:
    // red = B8, green = A8, blue = B8. Columns are listed column-major.
    let (dims, v) = values(&mut e, "imfuse(A, B)");
    assert_eq!(dims, [2, 2, 3]);
    assert_eq!(
        v,
        [0., 128., 0., 255., 0., 200., 100., 255., 0., 128., 0., 255.]
    );
    assert_eq!(text(&mut e, "class(imfuse(A, B))"), "uint8");
    // Spelled out: the same.
    let (_, w) = values(
        &mut e,
        "imfuse(A, B, 'falsecolor', 'Scaling', 'independent', 'ColorChannels', 'green-magenta')",
    );
    assert_eq!(v, w);
}

#[test]
fn color_channel_variants() {
    let mut e = Engine::default();
    run(
        &mut e,
        "A = uint8([0 100; 200 255]); B = uint8([50 50; 150 250]);",
    );
    // A8 = [0 100; 200 255] -> column-major [0 200 100 255];
    // B8 = [0 0; 128 255]   -> column-major [0 128 0 255].
    let a8 = [0., 200., 100., 255.];
    let b8 = [0., 128., 0., 255.];
    let zero = [0.; 4];
    let cat = |p: [&[f64; 4]; 3]| p.iter().flat_map(|x| x.iter().copied()).collect::<Vec<_>>();
    // red-cyan = [1 2 2].
    assert_eq!(
        values(&mut e, "imfuse(A, B, 'ColorChannels', 'red-cyan')").1,
        cat([&a8, &b8, &b8])
    );
    assert_eq!(
        values(&mut e, "imfuse(A, B, 'ColorChannels', 'Red-C')").1,
        cat([&a8, &b8, &b8])
    );
    assert_eq!(
        values(&mut e, "imfuse(A, B, 'ColorChannels', [1 2 0])").1,
        cat([&a8, &b8, &zero])
    );
    assert_eq!(
        values(&mut e, "imfuse(A, B, 'colorchannels', [0; 0; 2])").1,
        cat([&zero, &zero, &b8])
    );
    assert_eq!(
        values(&mut e, "imfuse(A, B, ColorChannels=[1 1 1])").1,
        cat([&a8, &a8, &a8])
    );
    assert!(
        error(&mut e, "imfuse(A, B, 'ColorChannels', [1 2 3])").contains("ColorChannels must be")
    );
    assert!(
        error(&mut e, "imfuse(A, B, 'ColorChannels', [1 2])").contains("ColorChannels must be")
    );
    assert!(
        error(&mut e, "imfuse(A, B, 'ColorChannels', 'blue-yellow')")
            .contains("unknown ColorChannels")
    );
    // ColorChannels only affects falsecolor.
    assert_eq!(
        values(&mut e, "imfuse(A, B, 'blend', 'ColorChannels', [1 2 0])").1,
        values(&mut e, "imfuse(A, B, 'blend')").1
    );
}

#[test]
fn scaling_modes_on_each_method() {
    let mut e = Engine::default();
    run(&mut e, "A = uint8([20 40]); B = uint8([60 120]);");
    // independent: A -> (A-20)/20 = [0 1], B -> (B-60)/60 = [0 1].
    // joint: range 20..120 over both: A -> [0 0.2], B -> [0.4 1].
    // none: unit scale A/255, B/255.
    let cases: &[(&str, &[f64])] = &[
        // falsecolor green-magenta: [B8 A8 B8]
        (
            "'falsecolor', 'Scaling', 'independent'",
            &[0., 255., 0., 255., 0., 255.],
        ),
        // joint: A8 = [0 51], B8 = [102 255]
        (
            "'falsecolor', 'Scaling', 'joint'",
            &[102., 255., 0., 51., 102., 255.],
        ),
        (
            "'falsecolor', 'Scaling', 'none'",
            &[60., 120., 20., 40., 60., 120.],
        ),
        // blend (A+B)/2: independent [0 1]; joint [0.2 0.6] -> [51 153];
        // none [40 80].
        ("'blend'", &[0., 255.]),
        ("'blend', 'Scaling', 'joint'", &[51., 153.]),
        ("'blend', 'Scaling', 'none'", &[40., 80.]),
        // diff |A-B|: independent [0 0]; joint [0.4 0.8] -> [102 204];
        // none [40 80].
        ("'diff'", &[0., 0.]),
        ("'diff', 'Scaling', 'joint'", &[102., 204.]),
        ("'diff', 'Scaling', 'none'", &[40., 80.]),
        // montage [A B]: independent [0 255 0 255]; joint [0 51 102 255].
        ("'montage'", &[0., 255., 0., 255.]),
        ("'montage', 'Scaling', 'joint'", &[0., 51., 102., 255.]),
        ("'montage', 'Scaling', 'none'", &[20., 40., 60., 120.]),
        // checkerboard on 1x2: tiles of 1x1, (1,1) from A, (1,2) from B.
        ("'checkerboard', 'Scaling', 'none'", &[20., 120.]),
        ("'checkerboard', 'Scaling', 'joint'", &[0., 255.]),
        // Prefixes and case of option names and values.
        ("'BLEND', 'scal', 'J'", &[51., 153.]),
    ];
    for (options, expected) in cases {
        let (_, v) = values(&mut e, &format!("imfuse(A, B, {options})"));
        assert_eq!(v, *expected, "imfuse(A, B, {options})");
    }
    assert_eq!(values(&mut e, "imfuse(A, B, 'montage')").0, [1, 4]);
    assert_eq!(values(&mut e, "imfuse(A, B, 'blend')").0, [1, 2]);
    assert_eq!(
        values(&mut e, "imfuse(A, B, 'blend', Scaling=\"joint\")").1,
        [51., 153.]
    );
}

#[test]
fn checkerboard_tiles_are_an_eighth_of_each_side() {
    let mut e = Engine::default();
    // 2x2: tiles of 1x1 -> [A(1,1) B(1,2); B(2,1) A(2,2)].
    run(
        &mut e,
        "A = uint8([10 20; 30 40]); B = uint8([50 60; 70 80]);",
    );
    let (dims, v) = values(&mut e, "imfuse(A, B, 'checkerboard', 'Scaling', 'none')");
    assert_eq!(dims, [2, 2]);
    assert_eq!(v, [10., 70., 60., 40.]);
    // 16x16 of 0 (A) and 255 (B): tiles of ceil(16/8) = 2 pixels.
    run(
        &mut e,
        "C = imfuse(zeros(16, 'uint8'), 255 * ones(16, 'uint8'), 'checkerboard', 'Scaling', 'none');",
    );
    for (r, c, want) in [
        (1, 1, 0.),
        (2, 2, 0.),
        (1, 3, 255.),
        (2, 4, 255.),
        (3, 1, 255.),
        (3, 3, 0.),
        (16, 16, 0.),
        (15, 13, 255.),
    ] {
        assert_eq!(scalar(&mut e, &format!("C({r}, {c})")), want, "C({r}, {c})");
    }
    // 17 rows: tiles of ceil(17/8) = 3 rows; row 3 is still A, row 4 is B.
    run(
        &mut e,
        "C = imfuse(zeros(17, 1, 'uint8'), 255 * ones(17, 1, 'uint8'), 'checkerboard', 'Scaling', 'none');",
    );
    assert_eq!(scalar(&mut e, "C(3)"), 0.);
    assert_eq!(scalar(&mut e, "C(4)"), 255.);
    assert_eq!(scalar(&mut e, "C(7)"), 0.);
}

#[test]
fn different_sizes_pad_with_zeros_at_the_bottom_and_right() {
    let mut e = Engine::default();
    // A 1x3 and B 2x1 on a 2x3 canvas: A -> [10 20 30; 0 0 0],
    // B -> [40 0 0; 50 0 0]; montage [A B] is 2x6.
    let (dims, v) = values(
        &mut e,
        "imfuse(uint8([10 20 30]), uint8([40; 50]), 'montage', 'Scaling', 'none')",
    );
    assert_eq!(dims, [2, 6]);
    assert_eq!(v, [10., 0., 20., 0., 30., 0., 40., 50., 0., 0., 0., 0.]);
    // Padding comes before independent scaling: A = [100 200] padded to
    // [100 200; 0 0] spans 0..200 -> [0.5 1; 0 0] -> [128 255; 0 0];
    // B = [50; 150] padded to [50 0; 150 0] spans 0..150 ->
    // [1/3 0; 1 0] -> [85 0; 255 0]. Falsecolor [B8 A8 B8].
    let (dims, v) = values(&mut e, "imfuse(uint8([100 200]), uint8([50; 150]))");
    assert_eq!(dims, [2, 2, 3]);
    assert_eq!(
        v,
        [85., 255., 0., 0., 128., 0., 255., 0., 85., 255., 0., 0.]
    );
    // An int16 zero is the middle of the unit scale: (0 + 32768) / 65535.
    let (_, v) = values(
        &mut e,
        "imfuse(int16(-32768), int16(zeros(1, 2)), 'montage', 'Scaling', 'none')",
    );
    // A = [0 pad] -> [0 128]; B = [128 128] (32768/65535*255 = 127.502).
    assert_eq!(v, [0., 128., 128., 128.]);
}

#[test]
fn input_classes_and_color_images() {
    let mut e = Engine::default();
    // double on the unit scale, values outside [0, 1] clip.
    assert_eq!(
        values(
            &mut e,
            "imfuse([0 0.5 1], [1 0.5 0], 'blend', 'Scaling', 'none')"
        )
        .1,
        [128., 128., 128.]
    );
    assert_eq!(
        values(
            &mut e,
            "imfuse([0 0.5 1], [1 0.5 0], 'diff', 'Scaling', 'none')"
        )
        .1,
        [255., 0., 255.]
    );
    assert_eq!(
        values(
            &mut e,
            "imfuse([-1 2], [0 0], 'montage', 'Scaling', 'none')"
        )
        .1,
        [0., 255., 0., 0.]
    );
    // single: (0.25 + 0.5) / 2 * 255 = 95.625 -> 96; (0.75 + 0.5) / 2 * 255 = 159.375 -> 159.
    assert_eq!(
        values(
            &mut e,
            "imfuse(single([0.25 0.75]), single([0.5 0.5]), 'blend', 'Scaling', 'none')"
        )
        .1,
        [96., 159.]
    );
    // logical: A = [1 0] -> [255 0], B = [0 1] -> [0 255]; falsecolor [B8 A8 B8].
    let (dims, v) = values(&mut e, "imfuse(logical([1 0]), logical([0 1]))");
    assert_eq!(dims, [1, 2, 3]);
    assert_eq!(v, [0., 255., 255., 0., 0., 255.]);
    assert_eq!(text(&mut e, "class(imfuse(true, false))"), "uint8");
    // uint16: 257 / 65535 * 255 = 1, 514 / 65535 * 255 = 2.
    assert_eq!(
        values(
            &mut e,
            "imfuse(uint16([0 65535]), uint16([257 514]), 'montage', 'Scaling', 'none')"
        )
        .1,
        [0., 255., 1., 2.]
    );
    // diff |0 - 1/255| = 1 level, |1 - 2/255| = 253 levels.
    assert_eq!(
        values(
            &mut e,
            "imfuse(uint16([0 65535]), uint16([257 514]), 'diff', 'Scaling', 'none')"
        )
        .1,
        [1., 253.]
    );
    // int16 extremes map to 0 and 1.
    assert_eq!(
        values(
            &mut e,
            "imfuse(int16([-32768 32767]), int16([0 0]), 'montage', 'Scaling', 'none')"
        )
        .1[..2],
        [0., 255.]
    );
    // Mixed classes: uint8 100 and double 0.5 on one scale; joint range
    // 100/255..0.5 -> A 0, B 1.
    assert_eq!(
        values(
            &mut e,
            "imfuse(uint8(100), 0.5, 'montage', 'Scaling', 'joint')"
        )
        .1,
        [0., 255.]
    );
    // A constant image is not stretched (independent): 7 stays 7.
    assert_eq!(
        values(&mut e, "imfuse(uint8([7 7]), uint8([0 255]), 'montage')").1,
        [7., 7., 0., 255.]
    );
    // RGB red (255 0 0) beside gray 100.
    run(&mut e, "R = uint8(cat(3, 255, 0, 0)); G = uint8(100);");
    // falsecolor: gray(R) = 0.298936021293775 -> 76.23 -> 76; B8 = 100.
    let (dims, v) = values(&mut e, "imfuse(R, G, 'Scaling', 'none')");
    assert_eq!(dims, [1, 1, 3]);
    assert_eq!(v, [100., 76., 100.]);
    // blend keeps colour; G is repeated: (255+100)/2 = 177.5 -> 178, 50, 50.
    let (dims, v) = values(&mut e, "imfuse(R, G, 'blend', 'Scaling', 'none')");
    assert_eq!(dims, [1, 1, 3]);
    assert_eq!(v, [178., 50., 50.]);
    // diff is on gray: |0.298936 - 100/255| * 255 = 23.77 -> 24.
    let (dims, v) = values(&mut e, "imfuse(R, G, 'diff', 'Scaling', 'none')");
    assert_eq!(dims, [1, 1]);
    assert_eq!(v, [24.]);
    // montage keeps colour: red [255 100], green [0 100], blue [0 100].
    let (dims, v) = values(&mut e, "imfuse(R, G, 'montage', 'Scaling', 'none')");
    assert_eq!(dims, [1, 2, 3]);
    assert_eq!(v, [255., 100., 0., 100., 0., 100.]);
    // checkerboard of two RGB images keeps colour.
    let (dims, _) = values(&mut e, "imfuse(R, R, 'checkerboard')");
    assert_eq!(dims, [1, 1, 3]);
}

#[test]
fn imfuse_errors() {
    let mut e = Engine::default();
    run(&mut e, "A = uint8([1 2]); B = uint8([3 4]);");
    for (code, message) in [
        ("imfuse(A)", "expected two images"),
        ("imfuse(A, B, 'sideways')", "unknown method 'sideways'"),
        ("imfuse(A, B, 'blend', 'Scaling')", "pairs"),
        (
            "imfuse(A, B, 'Scaling', 'bogus')",
            "unknown Scaling 'bogus'",
        ),
        ("imfuse(A, B, 'Scaling', 3)", "Scaling must be"),
        ("imfuse(A, B, 'Parent', 3)", "unknown option 'Parent'"),
        (
            "imfuse(int32(A), B)",
            "must be of class double, single, logical, uint8, uint16 or int16",
        ),
        (
            "imfuse(ones(2, 2, 2), B)",
            "grayscale (M-by-N) or truecolor",
        ),
        ("imfuse(A, zeros(0, 3))", "B must not be empty"),
        ("imfuse(A, 'text')", "expected a numeric or logical image"),
        ("imfuse(complex([1 2], [3 4]), B)", "complex"),
        (
            "imfuse(A, struct('ImageSize', [1 2]), B, struct('ImageSize', [1 2]))",
            "imfuse with spatial referencing objects is not supported yet",
        ),
        ("[C, RC] = imfuse(A, B)", "RC"),
    ] {
        let seen = error(&mut e, code);
        assert!(seen.contains(message), "{code}: {seen}");
    }
}

// imfuse(A, RA, B, RB): the output grid is the union of both world extents
// (public imfuse page) at RA's pixel extents; A and B are resampled onto it
// (identity, linear, 0 outside an input), then fused as imfuse(A, B).

#[test]
fn imfuse_with_matching_references_equals_plain_imfuse() {
    let mut e = Engine::default();
    run(
        &mut e,
        "A = uint8(mod((1:6)' * (1:7), 11) * 20); B = uint8(mod((1:6)' * (1:7) + 4, 13) * 15); \
         RA = imref2d(size(A)); RB = imref2d(size(B));",
    );
    for method in ["falsecolor", "blend"] {
        let code = format!(
            "C = imfuse(A, RA, B, RB, '{method}'); D = imfuse(A, B, '{method}'); \
             [C2, RC] = imfuse(A, RA, B, RB, '{method}');"
        );
        run(&mut e, &code);
        assert!(
            truth(&mut e, "isequal(C, D) && isequal(C2, D) && isa(C, 'uint8')"),
            "{method}"
        );
        assert!(truth(
            &mut e,
            "isequal(RC.ImageSize, [6 7]) && isequal(RC.XWorldLimits, [0.5 7.5]) && isequal(RC.YWorldLimits, [0.5 6.5])"
        ));
    }
    // imshowpair shows the same composite.
    run(&mut e, "h = imshowpair(A, RA, B, RB, 'blend');");
    assert!(truth(&mut e, "isequal(h.CData, imfuse(A, B, 'blend'))"));
}

#[test]
fn imfuse_with_shifted_reference_covers_the_union() {
    let mut e = Engine::default();
    // B's world limits move +10 in x and -5 in y. Both images hold 0 and 1
    // inside the overlap, so the independent scaling ranges are 0..1 both in
    // the union composite and in the composite of the overlap crops.
    run(
        &mut e,
        "A = mod((1:20)' * (1:30), 17) / 16; B = mod((1:20)' * (1:30) + 3, 19) / 18; \
         A(1, 11) = 0; A(2, 12) = 1; B(6, 1) = 0; B(7, 2) = 1; \
         RA = imref2d(size(A)); RB = imref2d(size(B), [10.5 40.5], [-4.5 15.5]);",
    );
    for method in ["falsecolor", "blend"] {
        run(
            &mut e,
            &format!("[C, RC] = imfuse(A, RA, B, RB, '{method}');"),
        );
        assert_eq!(
            values(&mut e, "[size(C, 1) size(C, 2)]").1,
            [25., 40.],
            "{method}"
        );
        assert!(truth(
            &mut e,
            "isequal(RC.ImageSize, [25 40]) && isequal(RC.XWorldLimits, [0.5 40.5]) && isequal(RC.YWorldLimits, [-4.5 20.5])"
        ));
        // Overlap: world x 10.5..30.5, y 0.5..15.5 = C rows 6:20, cols 11:30,
        // A(1:15, 11:30) and B(6:20, 1:20).
        run(
            &mut e,
            &format!("O = imfuse(A(1:15, 11:30), B(6:20, 1:20), '{method}');"),
        );
        assert!(truth(&mut e, "isequal(C(6:20, 11:30, :), O)"), "{method}");
        // Top-left corner (y < 0.5, x < 10.5) lies outside both images.
        assert!(
            truth(&mut e, "~any(reshape(C(1:5, 1:10, :), [], 1))"),
            "{method}"
        );
    }
}

#[test]
fn imfuse_with_matching_non_unit_references_equals_plain_imfuse() {
    let mut e = Engine::default();
    // RA = RB with pixel extents 0.7 by 1.3: both inputs already lie on the
    // union grid, so neither is resampled and the composite is imfuse(A, B)
    // to the last level. Levels 0..170 put every odd level on a rounding tie
    // of the independent scaling (255 / 170 = 1.5) and every odd sum on a tie
    // of 'blend', so a resample that is off by one ulp would show.
    run(
        &mut e,
        "A = uint8(mod((1:20)' * (1:30) * 7, 171)); B = uint8(mod((1:20)' * (1:30) * 5 + 3, 171)); \
         A(1, 1) = 0; A(2, 1) = 170; B(1, 1) = 0; B(2, 1) = 170; \
         RA = imref2d(size(A), 0.7, 1.3); RB = RA; \
         D = imfuse(A, B, 'blend', 'Scaling', 'none'); G = imfuse(A, B);",
    );
    run(
        &mut e,
        "[C, RC] = imfuse(A, RA, B, RB, 'blend', 'Scaling', 'none'); F = imfuse(A, RA, B, RB);",
    );
    assert_eq!(scalar(&mut e, "sum(C(:) ~= D(:))"), 0., "blend");
    assert_eq!(scalar(&mut e, "sum(F(:) ~= G(:))"), 0., "falsecolor");
    assert!(truth(&mut e, "isequal(C, D) && isequal(F, G)"));
    // RC is RA itself, not limits rebuilt from the pixel count.
    assert!(truth(
        &mut e,
        "isequal(RC.ImageSize, [20 30]) && isequal(RC.XWorldLimits, RA.XWorldLimits) && isequal(RC.YWorldLimits, RA.YWorldLimits)"
    ));
    // Limits one ulp away are the same grid: still no resampling.
    run(
        &mut e,
        "RN = imref2d(size(B), RA.XWorldLimits + [0 4e-15], RA.YWorldLimits - [2e-16 0]); \
         C = imfuse(A, RA, B, RN, 'blend', 'Scaling', 'none'); F = imfuse(A, RA, B, RN);",
    );
    assert!(truth(
        &mut e,
        "~isequal(RN.XWorldLimits, RA.XWorldLimits) && ~isequal(RN.YWorldLimits, RA.YWorldLimits)"
    ));
    assert!(truth(&mut e, "isequal(C, D) && isequal(F, G)"));
}

#[test]
fn imfuse_reference_union_is_admitted_before_allocation() {
    // 1 MiB memory budget (131072 8-byte units), no evaluation budget: the
    // setting of tests/limits_admission.rs. RB 7000 px away makes a
    // 7002-by-7002 union (49 M elements, under the element limit); the
    // preflight refuses it before any grid-sized buffer exists ("Statement
    // exceeds ..."), not after both inputs were resampled ("two padded ...").
    let mut e = Engine::with_limits(Limits {
        evaluation_units: None,
        memory_bytes: 1024 * 1024,
        ..Limits::browser()
    });
    run(
        &mut e,
        "A = ones(2); B = ones(2) / 2; RA = imref2d([2 2]); \
         RB = imref2d([2 2], [7000.5 7002.5], [7000.5 7002.5]);",
    );
    let message = error(&mut e, "n = numel(imfuse(A, RA, B, RB));");
    assert!(
        message.contains("Statement exceeds the 1 MiB memory limit"),
        "{message}"
    );
    assert!(!message.contains("two padded"), "{message}");
    let message = error(&mut e, "imshowpair(A, RA, B, RB)");
    assert!(message.contains("Statement exceeds"), "{message}");
    // A union that fits is still fused under the same budget.
    run(
        &mut e,
        "RB = imref2d([2 2], [10.5 12.5], [10.5 12.5]); C = imfuse(A, RA, B, RB);",
    );
    assert_eq!(
        values(&mut e, "[size(C, 1) size(C, 2) size(C, 3)]").1,
        [12., 12., 3.]
    );
}

#[test]
fn imfuse_reference_errors() {
    let mut e = Engine::default();
    run(
        &mut e,
        "A = ones(3, 4); RA = imref2d([3 4]); RW = imref2d([5 5]);",
    );
    assert!(error(&mut e, "imfuse(A, RW, A, RA)").contains("RA.ImageSize [5 5] must match"));
    assert!(error(&mut e, "imfuse(A, RA, A, RW)").contains("RB.ImageSize [5 5] must match"));
    assert!(error(&mut e, "[a, b, c] = imfuse(A, RA, A, RA)").contains("too many output"));
}

#[test]
fn imfuse_reference_span_that_overflows_is_an_error() {
    let mut e = Engine::default();
    // Finite limits whose span (2e308) is not: the pixel extent is inf. This
    // is an error, not a 2-by-1 composite of fill.
    run(
        &mut e,
        "A = ones(2); RA = imref2d([2 2]); \
         RX = imref2d([2 2], [-1e308 1e308], [0.5 2.5]); \
         RY = imref2d([2 2], [0.5 2.5], [-1e308 1e308]);",
    );
    for call in [
        "C = imfuse(A, RX, A, RX);",
        "[C, RC] = imfuse(A, RY, A, RY, 'blend', 'Scaling', 'none');",
        "imshowpair(A, RX, A, RX)",
    ] {
        let message = error(&mut e, call);
        assert!(
            message.contains("does not fit a finite grid"),
            "{call}: {message}"
        );
    }
    assert!(!e.variables.contains_key("C") && !e.variables.contains_key("RC"));
    // Only RB overflows: the count of RA's pixels is what is not finite.
    assert!(error(&mut e, "imfuse(A, RA, A, RX)").contains("spans too many pixels"));
    // Finite extent, but the last pixel ends past the largest double.
    run(
        &mut e,
        "RL = imref2d([2 2], [0 1e308], [0.5 2.5]); RM = imref2d([2 2], [0 1.7e308], [0.5 2.5]);",
    );
    assert!(error(&mut e, "imfuse(A, RL, A, RM)").contains("does not fit a finite grid"));
}

#[test]
fn imfuse_through_a_function_handle_takes_rgb_input() {
    let mut e = Engine::default();
    run(
        &mut e,
        "f = @imfuse; C = f(uint8(cat(3, 255, 0, 0)), uint8(100), 'blend', 'Scaling', 'none');",
    );
    assert_eq!(values(&mut e, "C").1, [178., 50., 50.]);
}

// ------------------------------------------------------------------- imshowpair

#[test]
fn imshowpair_shows_the_fused_image_and_returns_its_handle() {
    let mut e = Engine::default();
    run(
        &mut e,
        "A = uint8([0 100; 200 255]); B = uint8([50 50; 150 250]);",
    );
    run(&mut e, "h = imshowpair(A, B); C = imfuse(A, B);");
    assert_eq!(text(&mut e, "get(h, 'Type')"), "image");
    assert!(truth(&mut e, "isequal(double(get(h, 'CData')), double(C))"));
    {
        let axes = &e.figure.as_ref().unwrap().axes;
        assert_eq!(axes.extras.images.len(), 1);
        let image = &axes.extras.images[0];
        assert_eq!((image.rows, image.cols), (2, 2));
        assert!(axes.extras.equal_aspect && axes.extras.axis_off);
    }
    // Methods and options are imfuse's; the display holds the same pixels.
    run(
        &mut e,
        "h = imshowpair(A, B, 'diff', 'Scaling', 'joint'); D = imfuse(A, B, 'diff', 'Scaling', 'joint');",
    );
    assert!(truth(&mut e, "isequal(double(h.CData), double(D))"));
    assert_eq!(values(&mut e, "h.CData").0, [2, 2]);
    run(
        &mut e,
        "h = imshowpair(A, B, 'montage', 'Interpolation', 'bilinear');",
    );
    assert_eq!(values(&mut e, "h.CData").0, [2, 4]);
}

#[test]
fn imshowpair_without_outputs_sets_no_ans_and_draws_into_parent() {
    let mut e = Engine::default();
    run(&mut e, "imshowpair(rand(4), rand(4), 'blend')");
    assert!(e.array("ans").is_none() && !e.variables.contains_key("ans"));
    {
        let axes = &e.figure.as_ref().unwrap().axes;
        assert_eq!(axes.extras.images.len(), 1);
    }
    run(
        &mut e,
        "figure; ax = gca; h = imshowpair(uint8([1 2]), uint8([3 4]), 'montage', 'Parent', ax, 'Scaling', 'none');",
    );
    assert!(truth(&mut e, "isequal(get(h, 'Parent'), ax)"));
    assert_eq!(values(&mut e, "h.CData").1, [1., 2., 3., 4.]);
    assert!(error(&mut e, "imshowpair(1, 2, 'Parent', 5)").contains("Parent must be an axes"));
    assert!(
        error(&mut e, "imshowpair(1, 2, 'Interpolation', 'cubic')")
            .contains("unknown Interpolation")
    );
    assert!(error(&mut e, "[a, b] = imshowpair(1, 2)").contains("too many output"));
    assert!(
        error(&mut e, "imshowpair(1, struct(), 2, struct())")
            .contains("imshowpair with spatial referencing objects is not supported yet")
    );
}

#[cfg(feature = "plot-export")]
#[test]
fn imshowpair_figure_saves_without_a_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    run(
        &mut e,
        "imshowpair(uint8(magic(8) * 4), uint8(magic(8)' * 4), 'falsecolor'); saveas(gcf, 'pair.png');",
    );
    let png = image::open(dir.path().join("pair.png")).unwrap().to_rgb8();
    // Somewhere in the picture a green or magenta pixel shows a difference.
    assert!(png.pixels().any(|p| p.0[0] != p.0[1]), "no coloured pixels");
}
