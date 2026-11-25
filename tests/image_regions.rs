//! imhist, stretchlim, imadjust, graythresh, otsuthresh, imbinarize, strel,
//! imdilate/imerode/imopen/imclose/imtophat/imbothat, bwmorph, imfill,
//! bwareaopen, imclearborder, bwlabel, bwconncomp, labelmatrix, regionprops.
//!
//! Expected values (no MATLAB evidence):
//! - tests/fixtures/image_regions/python.json from
//!   tool/refs/image_regions/make_refs.py (NumPy 2.5, SciPy 1.18, scikit-image
//!   0.26): imhist via np.histogram with the documented bin edges, Otsu via
//!   skimage threshold_otsu, morphology via scipy.ndimage (the script asserts
//!   SciPy's origin convention against the direct definition), hole filling
//!   via binary_fill_holes and skimage reconstruction, labels via
//!   ndimage.label relabelled to column-major first-pixel order, region
//!   properties via skimage regionprops. Conversions (row/col to x/y, 0-based
//!   to 1-based, bbox corners, the 1/12 pixel variance skimage leaves out of
//!   its inertia tensor) are done and documented in the script.
//! - tests/fixtures/image_regions/octave.json from make_refs.m (GNU Octave
//!   10.3, image package 2.20.1): strel shapes, bwmorph, bwlabel, imfill,
//!   bwareaopen, imclearborder, stretchlim/imadjust/graythresh second checks
//!   and regionprops (whose moments agree with the 1/12 normalisation).
//! - Hand-worked cases, marked "hand".
//!
//! Differences from the references, each asserted below:
//! - Octave's strel('line') rasterises non-diagonal angles differently (and
//!   errors at some); only the axis and diagonal angles are compared.
//! - Octave's regionprops Perimeter is the Vossepoel-Smeulders estimate and
//!   scikit-image's a weighted pixel count; MatRust follows the documented
//!   "distance between each adjoining pair of pixels around the border",
//!   checked against the chain length of Octave's bwboundaries trace.
//! - scikit-image's graythresh bins span [min, max] of the image, MatRust's
//!   the class range (as documented): agreement to one bin width.
use matrust::{Array, Engine};
use serde_json::Value as Json;

fn python() -> Json {
    serde_json::from_str(include_str!("fixtures/image_regions/python.json")).unwrap()
}
fn octave() -> Json {
    serde_json::from_str(include_str!("fixtures/image_regions/octave.json")).unwrap()
}
fn inputs() -> Json {
    serde_json::from_str(include_str!("fixtures/image_regions/inputs.json")).unwrap()
}

fn run(source: &str) -> Engine {
    let mut e = Engine::default();
    if let Err(err) = e.execute(source) {
        panic!("{source}\n{err}");
    }
    e
}

fn error(source: &str) -> String {
    Engine::default()
        .execute(source)
        .expect_err(&format!("{source} should fail"))
}

/// Size and column-major data of `expr` (converted with double()); text
/// before the last `;` runs first as statements.
fn eval(e: &mut Engine, expr: &str) -> (Vec<usize>, Vec<f64>) {
    // The last "; " outside brackets separates the setup statements.
    let mut depth = 0i32;
    let mut split = None;
    for (k, ch) in expr.char_indices() {
        match ch {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth -= 1,
            ';' if depth == 0 && expr[k..].starts_with("; ") => split = Some(k),
            _ => {}
        }
    }
    let (setup, expr) = match split {
        Some(k) => (&expr[..k + 1], &expr[k + 2..]),
        None => ("", expr),
    };
    if let Err(err) = e.execute(&format!("{setup} tq_ = double({expr});")) {
        panic!("{expr}\n{err}");
    }
    let m = e.variables.get("tq_").expect("tq_").clone();
    (m.dims().to_vec(), m.data().to_vec())
}
fn data(e: &mut Engine, expr: &str) -> Vec<f64> {
    eval(e, expr).1
}
fn text(e: &mut Engine, expr: &str) -> String {
    e.execute(&format!("tq_ = {expr};")).unwrap();
    match e.array("tq_") {
        Some(Array::Character(c)) => String::from_utf16_lossy(c.units()),
        other => panic!("{expr} is not text: {other:?}"),
    }
}

fn numbers(j: &Json) -> Vec<f64> {
    match j {
        Json::Array(a) => a.iter().flat_map(numbers).collect(),
        Json::Number(n) => vec![n.as_f64().unwrap()],
        Json::Bool(b) => vec![f64::from(u8::from(*b))],
        other => panic!("not numeric: {other}"),
    }
}
/// A fixture image {size, data} as (size, column-major data).
fn image(j: &Json) -> (Vec<usize>, Vec<f64>) {
    let size = numbers(&j["size"]).iter().map(|&v| v as usize).collect();
    (size, numbers(&j["data"]))
}
/// MATLAB source that assigns the fixture image to `name` with `class`
/// (in chunks: a statement holds at most 256 expression nodes).
fn assign(name: &str, j: &Json, class: &str) -> String {
    let (size, data) = image(j);
    let dims: Vec<String> = size.iter().map(|v| v.to_string()).collect();
    let mut src = format!("{name} = zeros(1, {});\n", data.len());
    for (k, chunk) in data.chunks(100).enumerate() {
        let list: Vec<String> = chunk.iter().map(|v| format!("{v}")).collect();
        let start = k * 100 + 1;
        src += &format!(
            "{name}({start}:{}) = [{}];\n",
            start + chunk.len() - 1,
            list.join(" ")
        );
    }
    src + &format!("{name} = {class}(reshape({name}, [{}]));\n", dims.join(" "))
}

fn close(actual: &[f64], expected: &[f64], tol: f64, what: &str) -> f64 {
    assert_eq!(actual.len(), expected.len(), "{what}: length");
    let mut worst = 0f64;
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        let err = (a - b).abs();
        assert!(
            err <= tol * b.abs().max(1.),
            "{what}[{i}]: {a} vs {b} (tolerance {tol})"
        );
        worst = worst.max(err);
    }
    worst
}
fn same(e: &mut Engine, expr: &str, j: &Json, what: &str) {
    let (size, expected) = image(j);
    let (dims, got) = eval(e, expr);
    assert_eq!(dims, size, "{what}: size");
    assert_eq!(got, expected, "{what}");
}

// ---------------------------------------------------------------- imhist

#[test]
fn imhist_counts_match_numpy_with_documented_edges() {
    let p = python();
    let i = inputs();
    let mut e = run(&(assign("g", &i["gray"], "uint8")
        + &assign("g16", &i["gray16"], "uint16")
        + &assign("d", &i["dbl"], "double")
        + &assign("b", &i["bw"], "logical")));
    let h = &p["hist"];
    assert_eq!(data(&mut e, "imhist(g)"), numbers(&h["uint8_256"]));
    assert_eq!(data(&mut e, "imhist(g, 10)"), numbers(&h["uint8_10"]));
    assert_eq!(data(&mut e, "imhist(g16)"), numbers(&h["uint16_256"]));
    assert_eq!(data(&mut e, "imhist(d)"), numbers(&h["double_256"]));
    assert_eq!(data(&mut e, "imhist(d, 10)"), numbers(&h["double_10"]));
    assert_eq!(data(&mut e, "imhist(b)"), numbers(&h["logical_2"]));
    // Bin locations: evenly spaced over the class range (hand).
    let (dims, x) = eval(&mut e, "[c, x] = imhist(g, 10); x");
    assert_eq!(dims, [10, 1]);
    close(
        &x,
        &(0..10).map(|k| 255. * k as f64 / 9.).collect::<Vec<_>>(),
        1e-15,
        "x uint8",
    );
    let x = data(&mut e, "[c, x] = imhist(g16); x");
    close(
        &x,
        &(0..256)
            .map(|k| 65535. * k as f64 / 255.)
            .collect::<Vec<_>>(),
        1e-15,
        "x uint16",
    );
    assert_eq!(data(&mut e, "[c, x] = imhist(b); x"), [0., 1.]);
    let x = data(&mut e, "[c, x] = imhist(d, 5); x");
    assert_eq!(x, [0., 0.25, 0.5, 0.75, 1.]);
    // Octave agrees on the n = 10 counts of the uint8 image (second check).
    let o = octave();
    assert_eq!(
        data(&mut e, "imhist(g, 10)"),
        numbers(&o["imhist_uint8_10"]["counts"])
    );
}

#[test]
fn imhist_hand_cases_and_statement_form() {
    let mut e = run("");
    // n = 3 on uint8: bins centred on 0, 127.5, 255 (hand).
    assert_eq!(
        data(&mut e, "imhist(uint8([0 1 2 255 128]), 3)"),
        [3., 1., 1.]
    );
    // Out-of-range doubles land in the end bins; NaN is not counted (hand).
    assert_eq!(
        data(&mut e, "imhist([0 0.25 0.5 0.75 1 NaN -1 2], 5)"),
        [2., 1., 1., 1., 2.]
    );
    // Lower bin edges are inclusive: 0.5 with 3 bins is the centre bin, and
    // 0.25 (an edge) goes up (hand).
    assert_eq!(data(&mut e, "imhist([0.25 0.5 0.75], 3)"), [0., 2., 1.]);
    // int16 range starts at -32768 (hand).
    assert_eq!(
        data(&mut e, "imhist(int16([-32768 0 32767]), 3)"),
        [1., 1., 1.]
    );
    assert_eq!(
        data(&mut e, "[c, x] = imhist(int16(0), 3); x"),
        [-32768., -0.5, 32767.]
    );
    // No outputs: drawn through stem, nothing assigned.
    run("figure; imhist(uint8([1 2 3 200]))");
    assert!(error("imhist(uint8(ones(2, 2, 3)))").contains("2-D"));
}

// ------------------------------------------------------- stretchlim/imadjust

#[test]
fn stretchlim_on_ramp_constant_and_rgb() {
    let mut e = run("ramp = uint8(0:255);");
    // hand: cumulative fraction k/256 first exceeds 0.01 at k = 3 and reaches
    // 0.99 at k = 254, so [2; 253]/255.
    assert_eq!(
        eval(&mut e, "stretchlim(ramp)"),
        (vec![2, 1], vec![2. / 255., 253. / 255.])
    );
    let o = octave();
    close(
        &data(&mut e, "stretchlim(ramp)"),
        &numbers(&o["stretchlim_ramp"]),
        1e-15,
        "octave ramp",
    );
    let i = inputs();
    e.execute(&assign("g", &i["gray"], "uint8")).unwrap();
    close(
        &data(&mut e, "stretchlim(g)"),
        &numbers(&o["stretchlim_gray"]),
        1e-15,
        "octave gray",
    );
    assert_eq!(data(&mut e, "stretchlim(uint8([7 7 7]))"), [0., 1.]);
    assert_eq!(data(&mut e, "stretchlim(ramp, 0)"), [0., 1.]);
    assert_eq!(
        data(&mut e, "stretchlim(uint8([0 10 20 30]), 0)"),
        [0., 30. / 255.]
    );
    // Scalar tol t means [t 1-t]; doubles use 256 bins (hand: k/9 falls in
    // bin round(255k/9); the fraction first exceeds 0.1 after the second
    // value (bin 28) and reaches 0.9 at the ninth (bin 227)).
    assert_eq!(
        data(&mut e, "stretchlim((0:9)/9, 0.1)"),
        [28. / 255., 227. / 255.]
    );
    // RGB: one column per channel (hand).
    e.execute("rgb = cat(3, ramp, 255 - ramp, uint8(50*ones(1, 256)));")
        .unwrap();
    let (dims, v) = eval(&mut e, "stretchlim(rgb)");
    assert_eq!(dims, [2, 3]);
    assert_eq!(v, [2. / 255., 253. / 255., 2. / 255., 253. / 255., 0., 1.]);
}

#[test]
fn imadjust_ramp_gamma_inversion_and_rgb() {
    let mut e = run("ramp = uint8(0:255);");
    let o = octave();
    // Default limits are stretchlim(I); Octave second check.
    assert_eq!(data(&mut e, "imadjust(ramp)"), numbers(&o["imadjust_ramp"]));
    // hand: out = (v - 51) * 255/153 rounded half away from zero, saturated.
    assert_eq!(
        data(
            &mut e,
            "imadjust(uint8([0 51 100 128 204 255]), [0.2 0.8], [0 1])"
        ),
        [0., 0., 82., 128., 255., 255.]
    );
    // hand: gamma 0.5 on [0.2 0.8].
    let got = data(&mut e, "imadjust(ramp, [0.2 0.8], [0 1], 0.5)");
    let want: Vec<f64> = (0..256)
        .map(|v| {
            let t = ((v as f64 / 255.).clamp(0.2, 0.8) - 0.2) / 0.6;
            (255. * t.sqrt()).round()
        })
        .collect();
    assert_eq!(got, want);
    // low_out > high_out inverts (hand and Octave).
    let inv = data(&mut e, "imadjust(ramp, [], [1 0])");
    assert_eq!(inv, (0..256).map(|v| 255. - v as f64).collect::<Vec<_>>());
    assert_eq!(inv, numbers(&o["imadjust_ramp_invert"]));
    assert_eq!(
        data(&mut e, "imadjust([0 0.5 1], [], [0.2 0.4])"),
        [0.2, 0.30000000000000004, 0.4]
    );
    assert_eq!(text(&mut e, "class(imadjust(ramp))"), "uint8");
    assert_eq!(text(&mut e, "class(imadjust(single([0 1])))"), "single");
    // RGB with 2-by-3 limits: channel c uses column c (hand).
    e.execute("rgb = cat(3, uint8([0 128 255]), uint8([0 128 255]), uint8([0 128 255]));")
        .unwrap();
    let (dims, v) = eval(&mut e, "imadjust(rgb, [0 0 0.5; 1 1 1], [0 1 0; 1 0 1])");
    assert_eq!(dims, [1, 3, 3]);
    assert_eq!(v, [0., 128., 255., 255., 127., 0., 0., 1., 255.]);
    assert!(
        error("imadjust(uint8(0:255), [0.5 0.5])").contains("low_in must be less than high_in")
    );
}

// ----------------------------------------------------------------- Otsu

#[test]
fn otsu_hand_cases() {
    let mut e = run("");
    // hand: counts [2 0 0 2]: the between-class variance is 2.25 after bins
    // 1, 2 and 3, their mean is 2, so T = (2-1)/3; em = 2.25/2.25 = 1.
    assert_eq!(data(&mut e, "otsuthresh([2 0 0 2])"), [1. / 3.]);
    assert_eq!(
        data(&mut e, "[t, em] = graythresh(uint8([0 0 255 255])); em"),
        [1.]
    );
    // hand: values 10 and 200: every boundary from bin 11 to 200 separates
    // them; the mean bin is 105.5, so the level is 104.5/255.
    assert_eq!(
        data(&mut e, "graythresh(uint8([10 10 10 200 200 200]))"),
        [104.5 / 255.]
    );
    // hand: three levels 0, 1, 2 (counts 1 2 1 of 3 bins): boundary after bin
    // 1 or 2 give the same variance 1/3; mean bin 1.5, level 0.25.
    assert_eq!(data(&mut e, "otsuthresh([1 2 1])"), [0.25]);
    // A single grey level: level 0, em 0.
    assert_eq!(
        data(&mut e, "[t, em] = graythresh(uint8([9 9 9])); [t em]"),
        [0., 0.]
    );
    assert!(error("otsuthresh([1 -2 3])").contains("nonnegative"));
}

#[test]
fn graythresh_agrees_with_scikit_image_to_one_bin() {
    let p = python();
    let o = octave();
    let i = inputs();
    let mut e = run(&(assign("bm", &i["bimodal"], "uint8")
        + &assign("ov", &i["overlap"], "uint8")
        + &assign("g", &i["gray"], "uint8")
        + &assign("d", &i["dbl"], "double")));
    let mut report = Vec::new();
    let mut worst_bins = 0f64;
    // (The uniform random `gray` image is left out: its criterion is nearly
    // flat, so first-maximum and mean-of-maxima rules land bins apart.)
    for (var, key, scale) in [
        ("ov", "overlap", 255f64),
        ("double(ov) / 255", "overlap_double", 1.),
    ] {
        let level = data(&mut e, &format!("graythresh({var})"))[0];
        let sk = p["otsu"][key].as_f64().unwrap();
        // skimage bins span [min, max] (width (max-min)/256); ours the class
        // range (width scale/255). One bin of either is allowed.
        let (_, values) = image(&i["overlap"]);
        let values: Vec<f64> = values.iter().map(|v| v * scale / 255.).collect();
        let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let width = (scale / 255.).max((hi - lo) / 256.);
        worst_bins = worst_bins.max((level * scale - sk).abs() / width);
        let diff = (level * scale - sk).abs();
        report.push(format!("{key}: {} vs {sk}", level * scale));
        assert!(
            diff <= width + 1e-12,
            "{key}: {} vs skimage {sk}",
            level * scale
        );
    }
    // Two separated clusters (largest low value vl, smallest high value vh):
    // every boundary in the gap maximises the between-class variance. The
    // documented tie rule takes the mean bin, level*255 = (vl + vh - 1)/2
    // (hand); scikit-image takes the first maximum, its threshold is vl.
    let (_, values) = image(&i["bimodal"]);
    let vl = values
        .iter()
        .copied()
        .filter(|&v| v < 120.)
        .fold(0., f64::max);
    let vh = values
        .iter()
        .copied()
        .filter(|&v| v >= 120.)
        .fold(255., f64::min);
    let level = data(&mut e, "graythresh(bm)")[0];
    assert_eq!(level * 255., (vl + vh - 1.) / 2.);
    assert_eq!(p["otsu"]["bimodal"].as_f64().unwrap(), vl);
    // Octave's graythresh, also Otsu on 256 bins, as a second check.
    for (var, key) in [("bm", "graythresh_bimodal"), ("g", "graythresh_gray")] {
        let level = data(&mut e, &format!("graythresh({var})"))[0];
        let oct = o[key].as_f64().unwrap();
        assert!(
            (level - oct).abs() <= 1. / 255. + 1e-12,
            "{key}: {level} vs octave {oct}"
        );
    }
    eprintln!("{} (worst {worst_bins:.3} bins)", report.join("; "));
}

#[test]
fn imbinarize_is_threshold_times_range() {
    let i = inputs();
    let mut e = run(&(assign("g", &i["gray"], "uint8")
        + &assign("g16", &i["gray16"], "uint16")
        + &assign("d", &i["dbl"], "double")));
    for (var, range) in [("g", "255"), ("g16", "65535"), ("d", "1")] {
        let a = data(&mut e, &format!("imbinarize({var})"));
        let b = data(
            &mut e,
            &format!("double({var}) > graythresh({var}) * {range}"),
        );
        assert_eq!(a, b, "{var}");
        let c = data(&mut e, &format!("imbinarize({var}, 'global')"));
        assert_eq!(a, c, "{var} global");
        assert_eq!(
            text(&mut e, &format!("class(imbinarize({var}))")),
            "logical"
        );
    }
    // int16: the range starts at -32768 (hand).
    assert_eq!(
        data(&mut e, "imbinarize(int16([-32768 -1 0 32767]), 0.5)"),
        [0., 0., 1., 1.]
    );
    assert_eq!(data(&mut e, "imbinarize([0.1 0.6 0.9], 0.5)"), [0., 1., 1.]);
    assert_eq!(
        data(&mut e, "imbinarize([0.1 0.6 0.9], [0 0.7 0.95])"),
        [1., 0., 0.]
    );
    assert_eq!(
        data(&mut e, "imbinarize(uint8([101 103 200]), 0.4)"),
        [0., 1., 1.]
    );
    assert_eq!(
        text(&mut e, "class(imbinarize(magic(3) / 9, 'adaptive'))"),
        "logical"
    );
    assert!(error("imbinarize(magic(3) / 9, [0.1 0.2])").contains("same size"));
}

// ---------------------------------------------------------------- strel

fn nhood(e: &mut Engine, call: &str) -> (Vec<usize>, Vec<f64>) {
    e.execute(&format!("tq_se = {call};")).unwrap();
    eval(e, "tq_se.Neighborhood")
}

fn call_text(args: &Json) -> String {
    let parts: Vec<String> = args
        .as_array()
        .unwrap()
        .iter()
        .map(|a| match a {
            Json::String(s) => format!("'{s}'"),
            other => {
                let v = numbers(other);
                if v.len() == 1 {
                    format!("{}", v[0])
                } else {
                    format!(
                        "[{}]",
                        v.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                }
            }
        })
        .collect();
    format!("strel({})", parts.join(", "))
}

#[test]
fn strel_shapes_match_octave() {
    let o = octave();
    let mut e = run("");
    let mut checked = 0;
    for case in o["strel"].as_array().unwrap() {
        let call = call_text(&case["args"]);
        let got = nhood(&mut e, &call);
        assert_eq!(got, image(&case["nhood"]), "{call}");
        checked += 1;
    }
    // squares 1-5, rectangles 4, diamonds 1-5, disks r = 1..10 (n = 0),
    // octagons 3/6/9 and the line cases Octave builds.
    assert!(checked >= 40, "only {checked} strel cases");
    // Shapes Octave cannot build (radius 0) by hand.
    assert_eq!(nhood(&mut e, "strel('diamond', 0)"), (vec![1, 1], vec![1.]));
    assert_eq!(nhood(&mut e, "strel('octagon', 0)"), (vec![1, 1], vec![1.]));
    assert_eq!(nhood(&mut e, "strel('disk', 0, 0)"), (vec![1, 1], vec![1.]));
}

#[test]
fn strel_lines_hand() {
    let mut e = run("");
    // 30 degrees, length 5: offsets t = -2..2 along x, y = round(t tan 30) (hand).
    assert_eq!(
        nhood(&mut e, "strel('line', 5, 30)"),
        (
            vec![3, 5],
            vec![0., 0., 1., 0., 0., 1., 0., 1., 0., 1., 0., 0., 1., 0., 0.]
        )
    );
    // 60 degrees is the transpose-like counterpart (major axis y).
    assert_eq!(
        nhood(&mut e, "strel('line', 5, 60)"),
        (
            vec![5, 3],
            vec![0., 0., 0., 1., 1., 0., 0., 1., 0., 0., 1., 1., 0., 0., 0.]
        )
    );
    // Symmetric about the centre for every angle and length (rotating by 180
    // degrees gives the same neighbourhood).
    for len in [2., 3., 6., 9., 12.] {
        for deg in [10., 20., 33., 75., 100., 120., 170.] {
            let (dims, v) = nhood(&mut e, &format!("strel('line', {len}, {deg})"));
            let mut r = v.clone();
            r.reverse();
            assert_eq!(v, r, "line {len} {deg} not symmetric ({dims:?})");
        }
    }
}

#[test]
fn strel_disk_periodic_line_shapes() {
    let mut e = run("");
    for n in [4, 6, 8] {
        for r in 1..=10i64 {
            let (dims, v) = nhood(&mut e, &format!("strel('disk', {r}, {n})"));
            let (h, w) = (dims[0] as i64, dims[1] as i64);
            assert_eq!(h, w, "disk {r} {n} square");
            let half = (h - 1) / 2;
            let at = |row: i64, col: i64| v[(col * h + row) as usize] != 0.;
            let mut members = 0;
            for row in 0..h {
                for col in 0..w {
                    let (y, x) = (half - row, col - half);
                    let m = at(row, col);
                    // Symmetric under 90-degree rotation and both flips.
                    assert_eq!(m, at(half - x, half + y), "disk {r} {n}: rotation");
                    assert_eq!(m, at(row, w - 1 - col), "disk {r} {n}: flip");
                    assert_eq!(m, at(h - 1 - row, col), "disk {r} {n}: flip");
                    // Contains the exact disk of radius r - 1 ...
                    if x * x + y * y <= (r - 1) * (r - 1) {
                        assert!(m, "disk {r} {n}: missing ({x}, {y})");
                    }
                    // ... and lies inside the disk of radius 1.1 r.
                    if m {
                        members += 1;
                        assert!(
                            ((x * x + y * y) as f64).sqrt() <= 1.1 * r as f64,
                            "disk {r} {n}: ({x}, {y}) outside 1.1 r"
                        );
                    }
                }
            }
            assert!(members > 0);
        }
    }
    // Default n is 4.
    assert_eq!(
        nhood(&mut e, "strel('disk', 5)"),
        nhood(&mut e, "strel('disk', 5, 4)")
    );
}

#[test]
fn strel_object_properties_and_errors() {
    let mut e = run("SE = strel('square', 3);");
    assert_eq!(text(&mut e, "class(SE)"), "strel");
    assert_eq!(data(&mut e, "SE.Dimensionality"), [2.]);
    assert_eq!(text(&mut e, "class(SE.Neighborhood)"), "logical");
    assert_eq!(data(&mut e, "isa(SE, 'strel')"), [1.]);
    assert_eq!(data(&mut e, "isstruct(SE)"), [0.]);
    let shown = e.execute("SE").unwrap().output.join("");
    assert!(shown.contains("strel with properties"), "{shown}");
    assert!(shown.contains("Neighborhood: [3x3 logical]"), "{shown}");
    assert!(!shown.contains("MatRust"), "{shown}");
    // strel(nhood), 'arbitrary', and plain matrices wherever an SE is.
    assert_eq!(
        nhood(&mut e, "strel([1 0 1])"),
        (vec![1, 3], vec![1., 0., 1.])
    );
    assert_eq!(
        nhood(&mut e, "strel('arbitrary', [0 1; 1 0])"),
        (vec![2, 2], vec![0., 1., 1., 0.])
    );
    assert_eq!(
        nhood(&mut e, "strel('rectangle', [2 3])"),
        (vec![2, 3], vec![1.; 6])
    );
    assert_eq!(nhood(&mut e, "strel(SE)"), (vec![3, 3], vec![1.; 9]));
    assert!(error("strel('ball', 3, 3)").contains("not supported yet"));
    assert!(error("strel('arbitrary', [1 1], [0 0])").contains("not supported yet"));
    assert!(error("strel('octagon', 4)").contains("multiple of 3"));
    assert!(error("strel('disk', 3, 5)").contains("0, 4, 6 or 8"));
    assert!(error("strel('blob', 3)").contains("unknown shape"));
}

// ------------------------------------------------------------ morphology

#[test]
fn grey_and_binary_morphology_match_scipy() {
    let p = python();
    let i = inputs();
    let mut e = run(&(assign("g", &i["gray"], "uint8") + &assign("b", &i["bw"], "logical")));
    let ops = [
        ("imdilate", "dilate", "g"),
        ("imerode", "erode", "g"),
        ("imopen", "open", "g"),
        ("imclose", "close", "g"),
        ("imtophat", "tophat", "g"),
        ("imbothat", "bothat", "g"),
        ("imdilate", "bw_dilate", "b"),
        ("imerode", "bw_erode", "b"),
        ("imopen", "bw_open", "b"),
        ("imclose", "bw_close", "b"),
    ];
    for (name, case) in p["morph"].as_object().unwrap() {
        e.execute(&assign("se", &case["se"], "logical")).unwrap();
        for (f, key, img) in ops {
            same(
                &mut e,
                &format!("{f}({img}, se)"),
                &case[key],
                &format!("{f} {name} {img}"),
            );
        }
        // A strel object gives the same result as its neighbourhood.
        same(&mut e, "imdilate(g, strel(se))", &case["dilate"], name);
    }
    assert_eq!(text(&mut e, "class(imdilate(g, ones(3)))"), "uint8");
    assert_eq!(text(&mut e, "class(imerode(b, ones(3)))"), "logical");
    assert_eq!(text(&mut e, "class(imtophat(b, ones(3)))"), "logical");
    // Doubles keep -Inf/+Inf padding only where nothing is in range (hand).
    assert_eq!(data(&mut e, "imdilate([1 5 2], [1 1])"), [1., 5., 5.]);
    assert_eq!(data(&mut e, "imerode([1 5 2], [1 1])"), [1., 2., 2.]);
    // Each plane of an N-D image separately (hand).
    let (dims, v) = eval(&mut e, "imdilate(cat(3, [1 0 0], [0 0 3]), [1 1 1])");
    assert_eq!(dims, [1, 3, 2]);
    assert_eq!(v, [1., 1., 0., 0., 3., 3.]);
    // Large image (600x500 with 49 offsets runs on several workers): equal
    // to the serial result of a 60-column crop away from the crop border.
    e.execute(
        "big = uint8(mod((1:600)' * (1:500), 251)); se = strel('disk', 4, 0); \
         full = imdilate(big, se); part = imdilate(big(:, 1:60), se); \
         ok = isequal(full(:, 1:56), part(:, 1:56));",
    )
    .unwrap();
    assert_eq!(data(&mut e, "ok"), [1.]);
}

/// NaN-ignoring max/min of the in-image samples of a window: MATLAB max/min
/// ignore NaN unless every value is NaN (documented max/min rule).
fn nan_extreme(v: &[f64], max: bool) -> f64 {
    let finite: Vec<f64> = v.iter().copied().filter(|x| !x.is_nan()).collect();
    if finite.is_empty() {
        f64::NAN
    } else if max {
        finite.iter().copied().fold(f64::NEG_INFINITY, f64::max)
    } else {
        finite.iter().copied().fold(f64::INFINITY, f64::min)
    }
}

fn same_with_nan(actual: &[f64], expected: &[f64], what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}: length");
    for (k, (a, x)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (a.is_nan() && x.is_nan()) || a == x,
            "{what}: element {k} is {a}, expected {x}"
        );
    }
}

#[test]
fn nan_neighbourhoods_follow_max_min_rule() {
    // 7x7 ramp I(r, c) = r + 7(c - 1) with a 3x3 NaN block at rows/cols 3:5.
    // A window gives NaN only when every sample is NaN; any finite sample
    // wins over NaN. The block is interior, so border windows hold finite
    // samples and the -Inf/+Inf padding never decides (hand reference below).
    let mut e = run("I = reshape(1:49, 7, 7); I(3:5, 3:5) = NaN;");
    let img: Vec<f64> = (0..49)
        .map(|k| {
            let (r, c) = (k % 7, k / 7);
            if (2..5).contains(&r) && (2..5).contains(&c) {
                f64::NAN
            } else {
                (k + 1) as f64
            }
        })
        .collect();
    // Full square (row/column pass) and a diamond (offset list); both are
    // symmetric, so the reflected element equals the element.
    let square: &[(isize, isize)] = &[
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (0, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ];
    let diamond: &[(isize, isize)] = &[(0, -1), (-1, 0), (0, 0), (1, 0), (0, 1)];
    for (se, offsets) in [
        ("strel('square', 3)", square),
        ("strel('diamond', 1)", diamond),
    ] {
        for (f, max) in [("imdilate", true), ("imerode", false)] {
            let expected: Vec<f64> = (0..49)
                .map(|k| {
                    let (r, c) = ((k % 7) as isize, (k / 7) as isize);
                    let window: Vec<f64> = offsets
                        .iter()
                        .map(|&(dr, dc)| (r + dr, c + dc))
                        .filter(|&(rr, cc)| (0..7).contains(&rr) && (0..7).contains(&cc))
                        .map(|(rr, cc)| img[(cc * 7 + rr) as usize])
                        .collect();
                    nan_extreme(&window, max)
                })
                .collect();
            let actual = data(&mut e, &format!("{f}(I, {se})"));
            same_with_nan(&actual, &expected, &format!("{f} {se}"));
            // Block centre: every sample NaN.
            assert!(actual[3 * 7 + 3].is_nan(), "{f} {se} centre");
        }
    }
    // Hand values at the block corner (3,3): the square reads the finite
    // 9 10 11 16 23, the diamond 10 and 16.
    assert_eq!(
        data(
            &mut e,
            "a = imdilate(I, ones(3)); b = imerode(I, ones(3)); [a(3,3) b(3,3)]"
        ),
        [23., 9.]
    );
    assert_eq!(
        data(
            &mut e,
            "se = strel('diamond', 1); a = imdilate(I, se); b = imerode(I, se); [a(3,3) b(3,3)]"
        ),
        [16., 10.]
    );
    // A one-pixel element over NaN, double and single.
    assert!(data(&mut e, "imdilate(NaN, strel('square', 1))")[0].is_nan());
    assert!(data(&mut e, "imerode(NaN, strel('square', 1))")[0].is_nan());
    assert!(data(&mut e, "imdilate(single(NaN), 1)")[0].is_nan());
    assert_eq!(text(&mut e, "class(imerode(single(NaN), 1))"), "single");
    // Padding stays a -Inf/+Inf sample: a window that leaves the image and
    // reads only NaN inside it gives the padding value (documented border
    // rule), a window fully inside gives NaN.
    same_with_nan(
        &data(&mut e, "imdilate([NaN NaN NaN], [1 1 1])"),
        &[f64::NEG_INFINITY, f64::NAN, f64::NEG_INFINITY],
        "dilate border",
    );
    same_with_nan(
        &data(&mut e, "imerode([NaN NaN NaN], [1 0 1])"),
        &[f64::INFINITY, f64::NAN, f64::INFINITY],
        "erode border",
    );
    // Opening/closing chain the folds: the first pass leaves NaN only at the
    // centre (4,4), which the second pass ignores. Hand: the erosion around
    // (4,4) is 9 16 23 / 10 NaN 38 / 11 20 27, so the opening there is 38;
    // the dilation is 23 30 39 / 12 NaN 40 / 27 34 41, so the closing is 12.
    assert_eq!(
        data(
            &mut e,
            "a = imopen(I, ones(3)); b = imclose(I, ones(3)); [a(4,4) b(4,4)]"
        ),
        [38., 12.]
    );
}

#[test]
fn bwmorph_matches_octave() {
    let o = octave();
    let i = inputs();
    let mut e = run(&(assign("b", &i["bw"], "logical") + &assign("h", &i["holes"], "logical")));
    let m = &o["bwmorph"];
    for op in [
        "remove", "clean", "majority", "dilate", "erode", "open", "close",
    ] {
        same(&mut e, &format!("bwmorph(b, '{op}')"), &m[op], op);
    }
    same(&mut e, "bwmorph(b, 'thin', 1)", &m["thin1"], "thin 1");
    same(&mut e, "bwmorph(b, 'thin', 2)", &m["thin2"], "thin 2");
    same(&mut e, "bwmorph(b, 'thin', Inf)", &m["thinInf"], "thin Inf");
    same(
        &mut e,
        "bwmorph(h, 'thin', Inf)",
        &m["holes_thinInf"],
        "holes thin Inf",
    );
    same(
        &mut e,
        "bwmorph(h, 'remove')",
        &m["holes_remove"],
        "holes remove",
    );
    same(
        &mut e,
        "bwmorph(h, 'majority', 2)",
        &m["holes_majority2"],
        "holes majority 2",
    );
}

#[test]
fn bwmorph_fill_and_errors_hand() {
    let mut e = run("");
    // 'fill': a 0 whose eight neighbours are 1 (documentation); a 0 with only
    // its four edge neighbours set stays (GNU Octave fills that one too).
    assert_eq!(
        data(&mut e, "bwmorph(logical([1 1 1; 1 0 1; 1 1 1]), 'fill')"),
        [1.; 9]
    );
    assert_eq!(
        data(&mut e, "bwmorph(logical([0 1 0; 1 0 1; 0 1 0]), 'fill')"),
        [0., 1., 0., 1., 0., 1., 0., 1., 0.]
    );
    assert_eq!(
        text(&mut e, "class(bwmorph([0 2; 1 0], 'dilate'))"),
        "logical"
    );
    assert!(error("bwmorph(true(3), 'skel', Inf)").contains("not supported yet"));
    assert!(error("bwmorph(true(3), 'wobble')").contains("unknown operation"));
    assert!(error("bwmorph(true(3, 3, 2), 'clean')").contains("2-D"));
}

#[test]
fn imfill_holes_match_scipy_and_octave() {
    let p = python();
    let o = octave();
    let i = inputs();
    let mut e = run(&(assign("h", &i["holes"], "logical") + &assign("g", &i["gray"], "uint8")));
    same(
        &mut e,
        "imfill(h, 'holes')",
        &p["fill"]["holes4"],
        "holes 4 scipy",
    );
    same(
        &mut e,
        "imfill(h, 'holes')",
        &o["imfill_holes"],
        "holes 4 octave",
    );
    same(
        &mut e,
        "imfill(h, 8, 'holes')",
        &p["fill"]["holes8"],
        "holes 8 scipy",
    );
    same(
        &mut e,
        "imfill(h, 8, 'holes')",
        &o["imfill_holes8"],
        "holes 8 octave",
    );
    same(&mut e, "imfill(g)", &p["fill"]["gray4"], "grey 4 skimage");
    same(&mut e, "imfill(g)", &o["imfill_gray"], "grey 4 octave");
    same(
        &mut e,
        "imfill(g, 8)",
        &p["fill"]["gray8"],
        "grey 8 skimage",
    );
    // Seeds (hand): the 3x3 hole of the lower object, by linear index and by
    // subscripts; a seed on a foreground pixel changes nothing.
    let base = data(&mut e, "h");
    let filled = data(&mut e, "imfill(h, [10 6])");
    let by_index = data(&mut e, "imfill(h, sub2ind(size(h), 10, 6))");
    assert_eq!(filled, by_index);
    let changed: Vec<usize> = (0..base.len()).filter(|&k| base[k] != filled[k]).collect();
    // Row 10, col 6 of the 14-row image is linear index 6*14 - 14 + 10 = 80;
    // the hole is rows 10-12 (1-based), cols 6-8, minus the island (11, 7).
    let want: Vec<usize> = [
        (10, 6),
        (11, 6),
        (12, 6),
        (10, 7),
        (12, 7),
        (10, 8),
        (11, 8),
        (12, 8),
    ]
    .iter()
    .map(|(r, c)| (c - 1) * 14 + r - 1)
    .collect::<std::collections::BTreeSet<_>>()
    .into_iter()
    .collect();
    assert_eq!(changed, want);
    assert_eq!(data(&mut e, "isequal(imfill(h, [2 2]), h)"), [1.]);
    assert!(error("imfill(true(3))").contains("interactive"));
    assert!(error("imfill(true(3), 'holes', 'x')").contains("unknown option"));
}

#[test]
fn bwareaopen_and_imclearborder() {
    let o = octave();
    let p = python();
    let i = inputs();
    let mut e = run(&(assign("h", &i["holes"], "logical") + &assign("g", &i["gray"], "uint8")));
    same(
        &mut e,
        "bwareaopen(h, 5)",
        &o["bwareaopen5"],
        "bwareaopen 5",
    );
    same(
        &mut e,
        "bwareaopen(h, 5, 4)",
        &o["bwareaopen5_4"],
        "bwareaopen 5 4",
    );
    same(
        &mut e,
        "imclearborder(h)",
        &o["imclearborder"],
        "imclearborder",
    );
    same(
        &mut e,
        "imclearborder(h, 4)",
        &o["imclearborder4"],
        "imclearborder 4",
    );
    // Grey images: I minus the reconstruction by dilation of the border
    // values (the documented algorithm, which "tends to reduce the overall
    // intensity"; skimage reconstruction). GNU Octave instead keeps I where it
    // differs from the reconstruction: both agree on which pixels are zero.
    same(
        &mut e,
        "imclearborder(g)",
        &p["fill"]["clearborder_gray8"],
        "imclearborder grey",
    );
    let ours = data(&mut e, "imclearborder(g) > 0");
    let theirs: Vec<f64> = numbers(&o["imclearborder_gray"]["data"])
        .iter()
        .map(|&v| f64::from(u8::from(v > 0.)))
        .collect();
    assert_eq!(ours, theirs, "imclearborder grey support");
    // hand: a diagonal pair is one object with 8-connectivity only.
    assert_eq!(
        data(&mut e, "bwareaopen(logical([1 0; 0 1]), 2)"),
        [1., 0., 0., 1.]
    );
    assert_eq!(
        data(&mut e, "bwareaopen(logical([1 0; 0 1]), 2, 4)"),
        [0.; 4]
    );
    assert_eq!(
        data(&mut e, "imclearborder(logical([0 0 0; 0 1 0; 0 0 1]))"),
        [0.; 9]
    );
    assert_eq!(
        data(&mut e, "imclearborder(logical([0 0 0; 0 1 0; 0 0 1]), 4)"),
        [0., 0., 0., 0., 1., 0., 0., 0., 0.]
    );
}

// --------------------------------------------------- connected components

#[test]
fn bwlabel_matches_scipy_relabelled() {
    let p = python();
    let o = octave();
    let i = inputs();
    let mut e = run(&assign("b", &i["bw"], "logical"));
    let l = &p["labels"];
    same(&mut e, "bwlabel(b, 4)", &l["l4"], "bwlabel 4");
    same(&mut e, "bwlabel(b)", &l["l8"], "bwlabel 8");
    // GNU Octave numbers 8-connected objects in another order: compare the
    // partitions (each label set equal up to renumbering).
    for (conn, key) in [(4, "bwlabel4"), (8, "bwlabel8")] {
        let ours = data(&mut e, &format!("bwlabel(b, {conn})"));
        let theirs = numbers(&o[key]["data"]);
        let mut map = std::collections::BTreeMap::new();
        for (a, b) in ours.iter().zip(&theirs) {
            assert_eq!((*a == 0.), (*b == 0.), "octave {conn}: foreground");
            if *a != 0. {
                let prev = map.insert(a.to_bits(), b.to_bits());
                assert!(
                    prev.is_none_or(|p| p == b.to_bits()),
                    "octave {conn}: partition"
                );
            }
        }
        let distinct: std::collections::BTreeSet<_> = map.values().collect();
        assert_eq!(distinct.len(), map.len(), "octave {conn}: one-to-one");
    }
    assert_eq!(data(&mut e, "[L, n] = bwlabel(b, 4); n"), numbers(&l["n4"]));
    assert_eq!(data(&mut e, "[L, n] = bwlabel(b); n"), numbers(&l["n8"]));
    assert_eq!(text(&mut e, "class(bwlabel(b))"), "double");
    // The bwlabel documentation example (8-connected default and conn 4).
    e.execute(
        "BW = logical([1 1 1 0 0 0 0 0; 1 1 1 0 1 1 0 0; 1 1 1 0 1 1 0 0; 1 1 1 0 0 0 1 0;\
         1 1 1 0 0 0 1 0; 1 1 1 0 0 0 1 0; 1 1 1 0 0 1 1 0; 1 1 1 0 0 0 0 0]);",
    )
    .unwrap();
    let l8 = data(&mut e, "bwlabel(BW)'");
    let l4 = data(&mut e, "bwlabel(BW, 4)'");
    let row8 = [
        1., 1., 1., 0., 0., 0., 0., 0., 1., 1., 1., 0., 2., 2., 0., 0., 1., 1., 1., 0., 2., 2., 0.,
        0., 1., 1., 1., 0., 0., 0., 2., 0., 1., 1., 1., 0., 0., 0., 2., 0., 1., 1., 1., 0., 0., 0.,
        2., 0., 1., 1., 1., 0., 0., 2., 2., 0., 1., 1., 1., 0., 0., 0., 0., 0.,
    ];
    assert_eq!(l8, row8);
    let row4: Vec<f64> = row8
        .iter()
        .enumerate()
        .map(|(k, &v)| if v == 2. && k / 8 >= 3 { 3. } else { v })
        .collect();
    assert_eq!(l4, row4);
}

#[test]
fn bwconncomp_fields_and_labelmatrix_round_trip() {
    let i = inputs();
    let mut e = run(&assign("b", &i["bw"], "logical"));
    e.execute("CC = bwconncomp(b); CC4 = bwconncomp(b, 4);")
        .unwrap();
    assert_eq!(data(&mut e, "CC.Connectivity"), [8.]);
    assert_eq!(data(&mut e, "CC4.Connectivity"), [4.]);
    assert_eq!(data(&mut e, "CC.ImageSize"), [16., 16.]);
    let n = data(&mut e, "CC.NumObjects")[0];
    assert_eq!(data(&mut e, "[L, n] = bwlabel(b); n"), [n]);
    assert_eq!(eval(&mut e, "size(CC.PixelIdxList)").1, [1., n]);
    // Lists are ascending column vectors of the pixels with that label.
    for k in 1..=n as usize {
        let (dims, list) = eval(&mut e, &format!("CC.PixelIdxList{{{k}}}"));
        assert_eq!(dims[1], 1);
        assert!(list.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(list, data(&mut e, &format!("find(bwlabel(b) == {k})")));
    }
    assert_eq!(
        data(&mut e, "isequal(double(labelmatrix(CC)), bwlabel(b))"),
        [1.]
    );
    assert_eq!(
        data(&mut e, "isequal(double(labelmatrix(CC4)), bwlabel(b, 4))"),
        [1.]
    );
    assert_eq!(text(&mut e, "class(labelmatrix(CC))"), "uint8");
    // More than 255 objects: uint16 (hand: a checkerboard of 4-connected pixels).
    e.execute("[X, Y] = meshgrid(1:40); cb = logical(mod(X + Y, 2)); C2 = bwconncomp(cb, 4);")
        .unwrap();
    assert_eq!(data(&mut e, "C2.NumObjects"), [800.]);
    assert_eq!(text(&mut e, "class(labelmatrix(C2))"), "uint16");
    // No objects: 1-by-0 list.
    e.execute("C0 = bwconncomp(false(2));").unwrap();
    assert_eq!(data(&mut e, "C0.NumObjects"), [0.]);
    assert_eq!(eval(&mut e, "size(C0.PixelIdxList)").1, [1., 0.]);
}

// ------------------------------------------------------------ regionprops

const PROPS: &str = "'Area', 'Centroid', 'BoundingBox', 'Extent', 'EquivDiameter', 'FilledArea', \
     'MajorAxisLength', 'MinorAxisLength', 'Eccentricity', 'Orientation', 'Perimeter', \
     'PixelIdxList', 'PixelList', 'MeanIntensity', 'MaxIntensity', 'MinIntensity', 'WeightedCentroid'";

#[test]
fn regionprops_matches_scikit_image_and_octave() {
    let p = python();
    let o = octave();
    let i = inputs();
    let mut e = run(&(assign("L", &i["shapes"], "double") + &assign("W", &i["weights"], "uint8")));
    e.execute(&format!("s = regionprops(L, W, {PROPS});"))
        .unwrap();
    assert_eq!(eval(&mut e, "size(s)").1, [3., 1.]);
    let mut worst = std::collections::BTreeMap::<&str, f64>::new();
    for (k, (sk, oc)) in p["regionprops"]
        .as_array()
        .unwrap()
        .iter()
        .zip(o["regionprops"].as_array().unwrap())
        .enumerate()
    {
        let k = k + 1;
        for prop in [
            "Area",
            "Centroid",
            "BoundingBox",
            "Extent",
            "EquivDiameter",
            "FilledArea",
            "MajorAxisLength",
            "MinorAxisLength",
            "Eccentricity",
            "MeanIntensity",
            "MaxIntensity",
            "MinIntensity",
            "WeightedCentroid",
        ] {
            let got = data(&mut e, &format!("s({k}).{prop}"));
            let a = close(
                &got,
                &numbers(&sk[prop]),
                1e-12,
                &format!("{prop} {k} skimage"),
            );
            let b = close(
                &got,
                &numbers(&oc[prop]),
                1e-12,
                &format!("{prop} {k} octave"),
            );
            let w = worst.entry(prop).or_default();
            *w = w.max(a).max(b);
        }
        // The square's orientation is undefined (equal moments): the
        // documented formula gives 0, as Octave does.
        let got = data(&mut e, &format!("s({k}).Orientation"));
        if k > 1 {
            close(
                &got,
                &numbers(&sk["Orientation"]),
                1e-9,
                &format!("Orientation {k}"),
            );
        }
        close(
            &got,
            &numbers(&oc["Orientation"]),
            1e-9,
            &format!("Orientation {k} octave"),
        );
        // skimage's own axis lengths lack the 1/12 pixel variance:
        // 16 * (lambda + 1/12) = Major^2 relates the two.
        let major = data(&mut e, &format!("s({k}).MajorAxisLength"))[0];
        let skm = sk["skimage_axis_major_length"].as_f64().unwrap();
        assert!(
            (major * major - (skm * skm + 16. / 12.)).abs() < 1e-9,
            "major {k}"
        );
        // Perimeter: the chain length of Octave's boundary trace.
        let per = data(&mut e, &format!("s({k}).Perimeter"))[0];
        close(
            &[per],
            &numbers(&oc["chain"]),
            1e-12,
            &format!("Perimeter {k}"),
        );
        assert_eq!(
            data(&mut e, &format!("s({k}).PixelIdxList")),
            numbers(&sk["PixelIdxList"])
        );
        let (dims, pl) = eval(&mut e, &format!("s({k}).PixelList"));
        let idx = numbers(&sk["PixelIdxList"]);
        assert_eq!(dims, [idx.len(), 2]);
        for (j, q) in idx.iter().enumerate() {
            let q = *q as usize - 1;
            assert_eq!(
                [pl[j], pl[idx.len() + j]],
                [(q / 40 + 1) as f64, (q % 40 + 1) as f64]
            );
        }
    }
    eprintln!("worst absolute errors: {worst:?}");
    assert_eq!(text(&mut e, "class(s(1).MaxIntensity)"), "uint8");
}

#[test]
fn regionprops_hand_values_and_inputs() {
    let mut e = run("sq = false(8); sq(2:7, 3:8) = true;");
    // hand: 6x6 square at rows 2-7, cols 3-8: centroid (5.5, 4.5), box
    // [2.5 1.5 6 6], variance (36-1)/12 + 1/12 = 3 per axis, so both axes
    // 2*sqrt(2)*sqrt(6) = 4*sqrt(3); chain length 20.
    e.execute("s = regionprops(sq, 'all');").unwrap();
    assert_eq!(data(&mut e, "s.Area"), [36.]);
    assert_eq!(data(&mut e, "s.Centroid"), [5.5, 4.5]);
    assert_eq!(data(&mut e, "s.BoundingBox"), [2.5, 1.5, 6., 6.]);
    close(
        &data(&mut e, "s.MajorAxisLength"),
        &[4. * 3f64.sqrt()],
        1e-14,
        "major",
    );
    close(
        &data(&mut e, "s.MinorAxisLength"),
        &[4. * 3f64.sqrt()],
        1e-14,
        "minor",
    );
    assert_eq!(data(&mut e, "s.Eccentricity"), [0.]);
    assert_eq!(data(&mut e, "s.Orientation"), [0.]);
    assert_eq!(data(&mut e, "s.Perimeter"), [20.]);
    assert_eq!(data(&mut e, "s.Extent"), [1.]);
    // hand: a horizontal 1x5 bar has orientation 0, a vertical one 90, the
    // anti-diagonal (rising to the right on screen) +45, the diagonal -45.
    assert_eq!(
        data(
            &mut e,
            "t = regionprops(true(1, 5), 'Orientation'); t.Orientation"
        ),
        [0.]
    );
    assert_eq!(
        data(
            &mut e,
            "t = regionprops(true(5, 1), 'Orientation'); t.Orientation"
        ),
        [90.]
    );
    close(
        &data(
            &mut e,
            "t = regionprops(logical(fliplr(eye(4))), 'Orientation'); t.Orientation",
        ),
        &[45.],
        1e-12,
        "45",
    );
    close(
        &data(
            &mut e,
            "t = regionprops(logical(eye(4)), 'Orientation'); t.Orientation",
        ),
        &[-45.],
        1e-12,
        "-45",
    );
    // hand: chain lengths of a single pixel (0), a 1x3 bar (out and back, 4)
    // and a diagonal pair (2*sqrt(2)).
    assert_eq!(
        data(&mut e, "t = regionprops(true, 'Perimeter'); t.Perimeter"),
        [0.]
    );
    assert_eq!(
        data(
            &mut e,
            "t = regionprops(true(1, 3), 'Perimeter'); t.Perimeter"
        ),
        [4.]
    );
    close(
        &data(
            &mut e,
            "t = regionprops(logical(eye(2)), 'Perimeter'); t.Perimeter",
        ),
        &[2. * 2f64.sqrt()],
        1e-15,
        "diag",
    );
    // FilledArea counts the filled holes (hand: a 3x3 ring).
    assert_eq!(
        data(
            &mut e,
            "t = regionprops(logical([1 1 1; 1 0 1; 1 1 1]), 'FilledArea', 'Area'); [t.FilledArea t.Area]"
        ),
        [9., 8.]
    );
    // Max/MinIntensity ignore NaN unless the whole region is NaN (MATLAB
    // max/min rule; hand): region 1 is all NaN, region 2 holds 3 and NaN.
    same_with_nan(
        &data(
            &mut e,
            "t = regionprops([1 1 2 2], [NaN NaN 3 NaN], 'MaxIntensity', 'MinIntensity'); \
             [t.MaxIntensity t.MinIntensity]",
        ),
        &[f64::NAN, 3., f64::NAN, 3.],
        "region intensity with NaN",
    );
    // Default properties, a CC struct input and a label matrix with an empty label.
    assert_eq!(
        text(&mut e, "strjoin(fieldnames(regionprops(sq))', ',')"),
        "Area,Centroid,BoundingBox"
    );
    assert_eq!(
        data(
            &mut e,
            "t = regionprops(bwconncomp(logical([1 0 1])), 'Area'); [t.Area]"
        ),
        [1., 1.]
    );
    assert_eq!(
        data(&mut e, "t = regionprops([0 3 3], 'Area'); [t.Area]"),
        [0., 0., 2.]
    );
    assert_eq!(eval(&mut e, "size(regionprops(false(3)))").1, [0., 1.]);
    // Property names in any case and as a cell array.
    assert_eq!(
        data(
            &mut e,
            "t = regionprops(sq, {'area', 'EXTENT'}); [t.Area t.Extent]"
        ),
        [36., 1.]
    );
}

/// Perimeter is traced inside the region's bounding box: the cost follows the
/// region sizes, not regions x image size (fix round 2).
#[test]
fn regionprops_perimeter_cost_follows_region_size_not_image_size() {
    // Checkerboard of 8 x 8 pixel tiles on 2000 x 2000: tile (i, j) is set
    // when i + j is odd, so 250 * 250 / 2 = 31250 tiles (hand), which touch
    // only diagonally and are separate regions with 4-connectivity. The
    // boundary chain of an 8 x 8 square has 4 * 7 = 28 unit steps (hand).
    let mut e = run("[x, y] = meshgrid(0:1999); a = mod(floor(x / 8), 2) == 0; \
         b = mod(floor(y / 8), 2) == 0; clear x y; BW = xor(a, b); clear a b; \
         L = bwlabel(BW, 4); clear BW;");
    assert_eq!(data(&mut e, "max(L(:))"), [31250.]);
    let start = std::time::Instant::now();
    e.execute("t = regionprops(L, 'Perimeter');").unwrap();
    let elapsed = start.elapsed().as_secs_f64();
    eprintln!("regionprops Perimeter, 31250 regions on 2000 x 2000: {elapsed:.3} s");
    let per = data(&mut e, "[t.Perimeter]");
    assert_eq!(per.len(), 31250);
    assert!(per.iter().all(|&p| p == 28.), "every tile has perimeter 28");
    // Before the fix every region filled a 2000 x 2000 mask (1.2e11 byte
    // writes, several seconds); traced inside the box it takes well under a
    // second.
    assert!(elapsed < 2., "31250 regions took {elapsed:.2} s");

    // The same shape gives the same chain length wherever it sits, also
    // against the image edges. Reference value: Octave bwboundaries chain
    // of the 11-pixel snake, 12 unit steps + 4 diagonal steps
    // (tool/refs/image_regions/snake_chain.m).
    let mut e = run(
        "S = logical([1 1 0 0; 1 0 0 0; 1 1 1 1; 0 0 0 1; 0 1 1 1]); BW = false(1000); \
         BW(1:5, 1:4) = S; BW(500:504, 300:303) = S; BW(996:1000, 997:1000) = S; \
         t = regionprops(BW, 'Perimeter'); big = [t.Perimeter]; \
         t = regionprops(S, 'Perimeter'); small = t.Perimeter;",
    );
    let big = data(&mut e, "big");
    let small = data(&mut e, "small")[0];
    close(&[small], &[12. + 4. * 2f64.sqrt()], 1e-12, "snake chain");
    assert_eq!(big, [small, small, small]);
}

// ----------------------------------------------------------------- errors

/// Cancellation raised `after_ms` into `code`: the time it took to stop.
fn stop_time(e: &mut Engine, code: &str, after_ms: u64) -> f64 {
    use std::sync::atomic::Ordering;
    let stop = e.cancellation();
    let watcher = std::thread::spawn({
        let stop = stop.clone();
        move || {
            std::thread::sleep(std::time::Duration::from_millis(after_ms));
            stop.store(true, Ordering::Relaxed);
        }
    });
    let start = std::time::Instant::now();
    let message = e
        .execute(code)
        .expect_err(&format!("{code} should be cancelled"));
    let seconds = start.elapsed().as_secs_f64();
    watcher.join().unwrap();
    stop.store(false, Ordering::Relaxed);
    assert!(message.contains("cancelled"), "{code}: {message}");
    seconds
}

/// Work cap and work-based cancellation polls of the morphology functions
/// (fix round 2).
#[test]
fn morphology_work_is_capped_and_cancellation_is_polled_by_work() {
    let mut e = run("I = zeros(4096); SE = strel('disk', 1000, 0); \
         members = sum(SE.Neighborhood(:));");
    let members = data(&mut e, "members")[0] as usize;
    assert_eq!(members, 3_141_549, "Gauss circle count for radius 1000");
    // 4096 x 4096 pixels x 3.1e6 members = 5e13 reads: refused up front, for
    // every function built on dilation and erosion.
    let start = std::time::Instant::now();
    for name in [
        "imdilate", "imerode", "imopen", "imclose", "imtophat", "imbothat",
    ] {
        let msg = e
            .execute(&format!("{name}(I, SE);"))
            .expect_err(&format!("{name} should be refused"));
        assert!(
            msg.contains(&format!("{name}: 16777216 pixels at "))
                && msg.contains("neighbourhood reads each")
                && msg.contains("-read limit"),
            "{name}: {msg}"
        );
    }
    let msg = e.execute("imdilate(I, SE);").unwrap_err();
    assert!(
        msg.contains(&format!(
            "pixels at {members} neighbourhood reads each need {} reads in total",
            16_777_216u64 * members as u64
        )),
        "{msg}"
    );
    // imopen = erosion + dilation: twice the reads per pixel.
    let msg = e.execute("imopen(I, SE);").unwrap_err();
    assert!(
        msg.contains(&format!("pixels at {} neighbourhood", 2 * members)),
        "{msg}"
    );
    // A full rectangle costs rows + cols reads per pixel (two passes):
    // 4096 x 4096 x 4002 = 6.7e10 reads is over the cap too.
    let msg = e
        .execute("imerode(I, strel('rectangle', [2001 2001]));")
        .unwrap_err();
    assert!(
        msg.contains("pixels at 4002 neighbourhood reads each"),
        "{msg}"
    );
    let refused = start.elapsed().as_secs_f64();
    assert!(refused < 10., "refusing took {refused:.2} s");

    // Normal sizes still work (hand: one pixel grows to the 81 members of
    // disk(5); erosion of an all-ones image keeps it, the border pads +Inf).
    let mut e = run("I = zeros(512); I(256, 256) = 1; SE = strel('disk', 5, 0);");
    assert_eq!(data(&mut e, "nnz(imdilate(I, SE))"), [81.]);
    assert_eq!(data(&mut e, "nnz(imerode(true(512), SE))"), [262144.]);
    assert_eq!(data(&mut e, "nnz(imopen(true(512), SE))"), [262144.]);

    // Under the cap but far too long to wait for (2.5e10 and 3.1e10 reads,
    // 4e9 reads for the rectangle): cancellation lands within one poll
    // (about 1e7 reads), also inside a single column and inside the
    // parallel split over columns.
    let mut e = run("SE = strel('disk', 1000, 0); R = strel('rectangle', [2001 1]);");
    e.execute("I = zeros(8000, 1);").unwrap();
    let one_column = stop_time(&mut e, "imdilate(I, SE);", 50);
    e.execute("I = zeros(100);").unwrap();
    let parallel = stop_time(&mut e, "imerode(I, SE);", 50);
    e.execute("I = zeros(2000000, 1);").unwrap();
    let rectangle = stop_time(&mut e, "imdilate(I, R);", 50);
    for (what, seconds) in [
        ("one column", one_column),
        ("parallel columns", parallel),
        ("rectangle", rectangle),
    ] {
        eprintln!("cancel after 50 ms, {what}: stopped after {seconds:.3} s");
        assert!(seconds < 2., "{what}: stopped after {seconds:.2} s");
    }
    // The engine works again after a cancellation.
    assert_eq!(
        data(&mut e, "nnz(imdilate(true(3), strel('square', 3)))"),
        [9.]
    );
}

#[test]
fn errors_name_the_problem() {
    // 3-D input where a 2-D image is required.
    let msg = error("bwlabel(true(2, 2, 2))");
    assert!(
        msg.ends_with("bwlabel: the image must be 2-D (got an N-D array)"),
        "{msg}"
    );
    assert!(error("regionprops(true(2, 2, 2))").contains("2-D"));
    assert!(error("imfill(true(2, 2, 2), 'holes')").contains("2-D"));
    // Bad connectivity.
    let msg = error("bwlabel(true(3), 6)");
    assert!(
        msg.ends_with("bwlabel: connectivity must be 4 or 8"),
        "{msg}"
    );
    assert!(error("bwconncomp(true(3), 26)").contains("connectivity must be 4 or 8"));
    assert!(error("imfill(true(3), 5, 'holes')").contains("connectivity"));
    // Unknown and later properties.
    let msg = error("regionprops(true(3), 'Foo')");
    assert!(
        msg.ends_with("regionprops: unknown property 'Foo'"),
        "{msg}"
    );
    assert!(error("regionprops(true(3), 'Solidity')").contains("not supported yet"));
    assert!(error("regionprops('table', true(3), 'Area')").contains("not supported yet"));
    assert!(error("regionprops(true(3), 'MeanIntensity')").contains("intensity image"));
    assert!(error("[a, b, c] = bwlabel(true(3))").contains("too many output"));
    assert!(error("imdilate(true(3))").contains("imdilate"));
}
