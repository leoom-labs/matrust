//! Reference misc corpus, stored as expected values. LONG flags are from
//! tests/fixtures/fn/misc-cases.txt; the captured fixture headers omit those flags.
//! No reference is contacted by this test. See docs/tasks/2026-10-24-corpora-report.md.
use matrust::{Array, Engine};

const REFERENCE: &str = include_str!("fixtures/fn/misc-expected.txt");
const LONG: &[usize] = &[
    30, 31, 32, 33, 68, 69, 70, 71, 72, 75, 76, 87, 88, 89, 90, 91, 101, 103, 104, 105, 108, 109,
    110, 111, 112, 114, 115, 116, 117, 118,
];

/// Every unsupported case is named: no implicit skips. Unsupported functions
/// receive the central N-D guard after an N-D value has been constructed.
const KNOWN: &[(usize, &str)] = &[(
    111,
    "N-D arrays are not supported yet: hsv2rgb result [1 1 3]",
)];

/// Cases with harness adjustments. Console/class/size/error comparisons remain
/// exact unless the table explicitly specifies a comparison rule.
const ERROR_MESSAGE: &str = "Compare Diagnostic.message exactly, excluding the engine's Line/column location wrapper; the oracle captured MException.message.";
const FFT_ROUNDING: &str = "RustFFT versus reference FFT: decimal tokens may differ by at most 4e-15 absolute; all whitespace, labels, class and dimensions remain byte-exact.";
const HARNESS: &[(usize, &str)] = &[
    (6, ERROR_MESSAGE),
    (7, ERROR_MESSAGE),
    (8, ERROR_MESSAGE),
    (14, ERROR_MESSAGE),
    (15, ERROR_MESSAGE),
    (22, ERROR_MESSAGE),
    (28, ERROR_MESSAGE),
    (29, ERROR_MESSAGE),
    (35, ERROR_MESSAGE),
    (41, ERROR_MESSAGE),
    (54, ERROR_MESSAGE),
    (55, ERROR_MESSAGE),
    (59, ERROR_MESSAGE),
    (67, ERROR_MESSAGE),
    (70, FFT_ROUNDING),
    (71, FFT_ROUNDING),
    (73, ERROR_MESSAGE),
    (79, ERROR_MESSAGE),
    (86, ERROR_MESSAGE),
    (93, ERROR_MESSAGE),
    (113, ERROR_MESSAGE),
    (119, ERROR_MESSAGE),
    (127, ERROR_MESSAGE),
    (134, ERROR_MESSAGE),
];

fn error_message(engine: &Engine, error: String) -> String {
    engine
        .diagnostic
        .as_ref()
        .map_or(error, |d| d.message.clone())
}

/// Only the two explicitly listed FFT cases use this rule. Column widths,
/// whitespace and all nonnumeric text are still compared exactly.
fn fft_console_matches(actual: &str, expected: &str) -> bool {
    let re = regex::Regex::new(r"[-+]?[0-9]+\.[0-9]+(?:[eE][-+]?[0-9]+)?").unwrap();
    let a: Vec<_> = re.find_iter(actual).collect();
    let b: Vec<_> = re.find_iter(expected).collect();
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    let (mut ai, mut bi) = (0, 0);
    for (a, b) in a.into_iter().zip(b) {
        if a.len() != b.len() || actual[ai..a.start()] != expected[bi..b.start()] {
            return false;
        }
        let av: f64 = a.as_str().parse().unwrap();
        let bv: f64 = b.as_str().parse().unwrap();
        if !av.is_finite() || !bv.is_finite() || (av - bv).abs() > 4e-15 {
            return false;
        }
        ai = a.end();
        bi = b.end();
    }
    actual[ai..] == expected[bi..]
}

struct Case<'a> {
    id: usize,
    statement: &'a str,
    output: &'a str,
    class_size: Option<&'a str>,
}

fn cases() -> Vec<Case<'static>> {
    let mut cases = Vec::new();
    let mut rest = REFERENCE;
    while let Some(start) = rest.find("@@CASE ") {
        let (header, after) = rest[start + 7..].split_once('\n').expect("header");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (block, after) = after.split_once("@@END\n").expect("end");
        let (output, class_size) = match block.split_once("@@CLASS ") {
            Some((output, class)) => (
                output,
                Some(class.strip_suffix('\n').expect("class newline")),
            ),
            None => (block, None),
        };
        cases.push(Case {
            id: id.parse().expect("numeric id"),
            statement,
            output,
            class_size,
        });
        rest = after;
    }
    cases
}

#[test]
fn corpus_is_complete_and_exceptions_are_explicit() {
    let cases = cases();
    assert_eq!(cases.len(), 134);
    for (i, case) in cases.iter().enumerate() {
        assert_eq!(case.id, i + 1);
        assert_eq!(case.class_size.is_none(), case.output.starts_with("ERR "));
        assert_eq!(
            case.class_size.is_none(),
            HARNESS.contains(&(case.id, ERROR_MESSAGE))
        );
    }
    for (i, &(id, reason)) in KNOWN.iter().enumerate() {
        assert!(cases.iter().any(|c| c.id == id));
        assert!(!reason.is_empty());
        assert!(!KNOWN[..i].iter().any(|&(n, _)| n == id));
    }
    for (i, &(id, rule)) in HARNESS.iter().enumerate() {
        assert!(cases.iter().any(|c| c.id == id));
        assert!(!rule.is_empty());
        assert!(!HARNESS[..i].iter().any(|&(n, _)| n == id));
        assert!(!KNOWN.iter().any(|&(n, _)| n == id));
    }
}

#[test]
fn fft_harness_only_allows_last_digit_roundoff() {
    let expected = "\nx =\n\n   1.000000000000000\n\n";
    assert!(fft_console_matches(
        "\nx =\n\n   1.000000000000001\n\n",
        expected
    ));
    assert!(!fft_console_matches(
        "\nx =\n\n   1.000000000100000\n\n",
        expected
    ));
    assert!(!fft_console_matches(
        "\nx =\n\n  1.000000000000000\n\n",
        expected
    ));
    assert!(!fft_console_matches("\nx =\n\n   1.0\n\n", expected));
    assert!(!fft_console_matches(
        "\ny =\n\n   1.000000000000000\n\n",
        expected
    ));
}

#[test]
fn misc_matches_reference() {
    let mut failures = Vec::new();
    let mut passed = 0;
    let mut known = 0;
    for case in cases() {
        let mut engine = Engine::default();
        if LONG.contains(&case.id) {
            engine.execute("format long").unwrap();
        }
        let result = engine.execute(case.statement);
        if let Some((_, reason)) = KNOWN.iter().find(|&&(id, _)| id == case.id) {
            // Actually execute KNOWN cases too. Success would make this entry stale.
            let expected = match case.id {
                111 => "N-D arrays are not supported yet",
                _ => "Only two-dimensional arrays and trailing singleton dimensions are supported",
            };
            let error = result.err().map(|e| error_message(&engine, e));
            if error.as_deref() != Some(expected) {
                failures.push(format!(
                    "KNOWN {} changed: expected {expected:?}, actual {error:?}; {reason}",
                    case.id
                ));
            }
            eprintln!("KNOWN {}: {reason}; actual: {error:?}", case.id);
            known += 1;
            continue;
        }
        let actual = match result {
            Ok(result) => result
                .output
                .iter()
                .map(|s| format!("{s}\n"))
                .collect::<String>(),
            Err(error) => {
                assert!(
                    HARNESS.contains(&(case.id, ERROR_MESSAGE)),
                    "Unlisted error comparison rule for {}",
                    case.id
                );
                format!("ERR {}\n", error_message(&engine, error))
            }
        };
        let actual_class = engine.array("x").map(|a| {
            format!(
                "{} [{}]",
                a.class(),
                a.dims()
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        });
        let console_matches = if HARNESS.contains(&(case.id, FFT_ROUNDING)) {
            fft_console_matches(&actual, case.output)
        } else {
            actual == case.output
        };
        if console_matches && actual_class.as_deref() == case.class_size {
            passed += 1;
        } else {
            failures.push(format!(
                "case {} `{}`\nexpected {:?}\nactual   {:?}\nclass expected {:?}, actual {:?}",
                case.id, case.statement, case.output, actual, case.class_size, actual_class
            ));
        }
    }
    assert_eq!(known, KNOWN.len());
    assert!(
        failures.is_empty(),
        "{passed}/{} supported cases passed; {known} KNOWN:\n{}",
        134 - known,
        failures.join("\n")
    );
    assert_eq!(passed + known, 134);
}

fn numeric(engine: &mut Engine, source: &str) -> Vec<f64> {
    engine.execute(source).unwrap();
    match engine.array("x").unwrap() {
        Array::Numeric(a) => a.data().to_vec(),
        Array::Typed(a) => a.to_f64_vec().unwrap(),
        Array::Logical(a) => a.data().iter().map(|&v| f64::from(v)).collect(),
        a => panic!("Expected numeric x, got {}", a.class()),
    }
}

#[test]
fn user_functions_shadow_misc_builtins_for_single_and_multiple_outputs() {
    for name in [
        "shiftdim",
        "inpolygon",
        "peaks",
        "hsv2rgb",
        "rgb2hsv",
        "bucky",
    ] {
        let mut e = Engine::default();
        e.execute(&format!(
            "[x,y] = {name}(10); z = {name}(20);\nfunction [a,b] = {name}(n)\na=n; b=n+1;\nend\n"
        ))
        .unwrap();
        assert_eq!(e.variables["x"].data(), &[10.], "{name}");
        assert_eq!(e.variables["y"].data(), &[11.], "{name}");
        assert_eq!(e.variables["z"].data(), &[20.], "{name}");
    }
}

#[test]
fn peaks_bare_and_filter2_conversion_and_hsv_sectors_and_polyarea_empty() {
    let mut e = Engine::default();
    assert_eq!(
        numeric(&mut e, "[X,Y,Z] = peaks; x = size(Z);"),
        vec![49., 49.]
    );
    assert_eq!(
        numeric(&mut e, "[X,Y,Z] = peaks(5); x = [size(X) size(Y) size(Z)];"),
        vec![5.; 6]
    );
    assert_eq!(
        numeric(&mut e, "x = filter2(ones(3), [1 0 1; 0 1 0; 1 0 1] > 0);"),
        numeric(
            &mut e,
            "x = filter2(ones(3), double([1 0 1; 0 1 0; 1 0 1]));"
        )
    );
    assert_eq!(e.array("x").unwrap().class(), "double");
    assert_eq!(
        numeric(&mut e, "x = filter2([1 2 1], uint8([100 200 250 3]));"),
        vec![400., 750., 703., 256.]
    );
    assert_eq!(e.array("x").unwrap().class(), "double");
    // Hue spans six equal sectors; the upper endpoint wraps to the first sector.
    // Values: v=1, s=0.5, hand-derived (p = 0.5 throughout; q, t from f = 6h - k).
    for (h, rgb) in [
        ("0.125", [1., 0.875, 0.5]),
        ("0.25", [0.75, 1., 0.5]),
        ("0.375", [0.5, 1., 0.625]),
        ("0.5625", [0.5, 0.8125, 1.]),
        ("0.75", [0.75, 0.5, 1.]),
        ("0.875", [1., 0.5, 0.875]),
        ("1", [1., 0.5, 0.5]),
    ] {
        assert_eq!(
            numeric(&mut e, &format!("x = hsv2rgb([{h} 0.5 1]);")),
            rgb.to_vec(),
            "hue {h}"
        );
    }
    // Reference behavior (checked 2026-10-24): a colormap value outside [0,1] is rejected.
    for map in ["[-0.125 1 1]", "[1.25 1 1]", "[0.5 1.2 1]"] {
        let error = e.execute(&format!("x = hsv2rgb({map});")).unwrap_err();
        assert!(
            error.ends_with("Valid colormaps cannot have values outside the range [0,1]."),
            "{map}: {error}"
        );
    }
    for (source, shape) in [
        ("x = polyarea(zeros(0,3), zeros(0,3));", (1, 3)),
        ("x = polyarea(zeros(3,0), zeros(3,0));", (1, 0)),
        ("x = polyarea([], []);", (0, 0)),
        ("x = polyarea(ones(2,3), ones(2,3), 3);", (2, 3)),
    ] {
        e.execute(source).unwrap();
        let a = e.array("x").unwrap();
        let expected = if source.contains("[], []") {
            (1, 1)
        } else {
            shape
        };
        assert_eq!((a.rows(), a.cols()), expected, "{source}");
    }
    assert_eq!(
        numeric(&mut e, "x = polyarea(zeros(0,3), zeros(0,3));"),
        vec![0.; 3]
    );
    assert_eq!(
        numeric(&mut e, "x = polyarea(ones(2,3), ones(2,3), 3);"),
        vec![0.; 6]
    );
}

#[test]
fn integer_bits_and_byte_swaps_keep_all_64_bits() {
    let mut e = Engine::default();
    e.execute("a = uint64(18446744073709551615); x = bitset(a, 64, 0);")
        .unwrap();
    let Array::Typed(x) = e.array("x").unwrap() else {
        panic!("typed")
    };
    assert_eq!(x.class().name(), "uint64");
    assert_eq!(x.int(0), (1i128 << 63) - 1);
    assert_eq!(numeric(&mut e, "x = bitget(a, [1 32 53 64]);"), vec![1.; 4]);
    for class in [
        "int8", "uint8", "int16", "uint16", "int32", "uint32", "int64", "uint64", "single",
        "double",
    ] {
        assert_eq!(
            numeric(
                &mut e,
                &format!("v = {class}([0 1 42]); x = isequal(swapbytes(swapbytes(v)), v);")
            ),
            vec![1.]
        );
    }
    assert_eq!(numeric(&mut e, "x = bitset(0, 8, 1, 'int8');"), vec![-128.]);
    // Supplementary forms: default uint64 assumption, also used by the
    // existing bitshift/bitcmp family; not part of the reference fixture.
    assert_eq!(numeric(&mut e, "x = bitget(2^63, 64);"), vec![1.]);
    assert_eq!(numeric(&mut e, "x = bitset(0, 64);"), vec![2f64.powi(63)]);
    assert_eq!(
        numeric(&mut e, "x = bitget(uint8(128), 8, 'uint8');"),
        vec![1.]
    );
    assert!(e.execute("bitget(2^64, 64)").is_err());
    assert!(e.execute("bitget(uint8(1), 1, 'int8')").is_err());
}

#[test]
fn hex_roundtrips_and_column_major_shapes() {
    let mut e = Engine::default();
    assert_eq!(
        numeric(&mut e, "x = hex2num(num2hex([1 3; 2 4]));"),
        vec![1., 2., 3., 4.]
    );
    assert_eq!(numeric(&mut e, "x = hex2num(['3ff';'4  ']);"), vec![1., 2.]);
    assert_eq!(
        numeric(&mut e, "x = hex2num('3 f');"),
        vec![f64::from_bits(0x30f0_0000_0000_0000)]
    );
    for source in ["x = hex2num('');", "x = hex2num({});"] {
        e.execute(source).unwrap();
        let a = e.array("x").unwrap();
        assert_eq!((a.rows(), a.cols()), (0, 0), "{source}");
    }
    assert!(e.execute("x = hex2num('3ff', 'single');").is_err());
    for source in ["x = NaN;", "x = nan;", "x = [NaN 1];"] {
        let bits: Vec<u64> = numeric(&mut e, source)
            .into_iter()
            .filter(|v| v.is_nan())
            .map(f64::to_bits)
            .collect();
        assert!(
            !bits.is_empty() && bits.iter().all(|&b| b == 0xfff8_0000_0000_0000),
            "{source}"
        );
    }
    e.execute("x = num2hex(NaN);").unwrap();
    assert_eq!(
        numeric(&mut e, "x = strcmp(num2hex(NaN), 'fff8000000000000');"),
        vec![1.]
    );
    assert_eq!(
        numeric(&mut e, "x = hex2num('8000000000000000');")[0].to_bits(),
        (-0f64).to_bits()
    );
    assert!(e.execute("x = hex2num('00000000000000000');").is_err());
    assert_eq!(
        numeric(
            &mut e,
            "x = strcmp(num2hex(hex2num('7ff8000000000042')), '7ff8000000000042');"
        ),
        vec![1.]
    );
    assert_eq!(
        numeric(
            &mut e,
            "x = strcmp(num2hex(hex2num('7ff8000000000000')), '7ff8000000000000');"
        ),
        vec![1.]
    );
}

#[test]
fn shape_operations_preserve_class_and_shared_identity() {
    let mut e = Engine::default();
    e.execute("a = uint64([1 2; 3 4]); b = permute(a, [1 2 3]); c = full(a);")
        .unwrap();
    let a = e.array("a").unwrap();
    assert!(a.same_snapshot(&e.array("b").unwrap()));
    assert!(a.same_snapshot(&e.array("c").unwrap()));
    assert_eq!(
        numeric(&mut e, "x = ipermute(permute(a, [2 1]), [2 1]);"),
        vec![1., 3., 2., 4.]
    );
    assert_eq!(e.array("x").unwrap().class(), "uint64");
    assert_eq!(
        numeric(&mut e, "[a,n] = shiftdim([1 2 3]); x = [a; n];"),
        vec![1., 2., 3., 1.]
    );
    e.execute("x = flipdim('abc', 2);").unwrap();
    let Array::Character(a) = e.array("x").unwrap() else {
        panic!("char")
    };
    assert_eq!(a.units(), &[99, 98, 97]);
    // N-D part 2: these 2-D inputs now give N-D results (MATLAB shiftdim/permute pages).
    for (source, dims, data) in [
        ("x = shiftdim([1 2], -1);", &[1, 1, 2][..], &[1., 2.][..]),
        (
            "x = permute([1 2; 3 4], [1 3 2]);",
            &[2, 1, 2],
            &[1., 3., 2., 4.],
        ),
        (
            "x = ipermute([1 2; 3 4], [1 3 2]);",
            &[2, 1, 2],
            &[1., 3., 2., 4.],
        ),
    ] {
        assert_eq!(numeric(&mut e, source), data, "{source}");
        assert_eq!(e.array("x").unwrap().dims().as_ref(), dims, "{source}");
    }
    let error = e.execute("hsv2rgb(1,1,1)").unwrap_err();
    assert_eq!(error_message(&e, error), "N-D arrays are not supported yet");
}

#[test]
fn geometric_boundaries_holes_and_color_roundtrip() {
    let mut e = Engine::default();
    assert_eq!(
        numeric(
            &mut e,
            "[a,b] = inpolygon([0 1 2 0.5], [0 1 1 0.5], [0 1 1 0], [0 0 1 1]); x = [a;b];"
        ),
        vec![1., 1., 1., 1., 0., 0., 1., 0.]
    );
    assert_eq!(
        numeric(
            &mut e,
            "x = inpolygon([0.5 1.5], [0.5 0.5], [0 2 2 0 NaN 1 1 1.6 1.6], [0 0 1 1 NaN 0.2 0.8 0.8 0.2]);"
        ),
        vec![1., 0.]
    );
    let actual = numeric(
        &mut e,
        "x = hsv2rgb(rgb2hsv([0.1 0.3 0.9; 1 0 0; 0.5 0.5 0.5]));",
    );
    for (a, b) in actual
        .iter()
        .zip([0.1, 1., 0.5, 0.3, 0., 0.5, 0.9, 0., 0.5])
    {
        assert!((a - b).abs() < 1e-14);
    }
    assert_eq!(
        numeric(&mut e, "[r,g,b] = hsv2rgb([0 1 1]); x = [r g b];"),
        vec![1., 0., 0.]
    );
    assert_eq!(
        numeric(&mut e, "[r,g] = hsv2rgb([0 1 1]); x = [r g];"),
        vec![1., 0.]
    );
}

#[test]
fn interpolation_of_periodic_complex_data_and_dimension_two() {
    let mut e = Engine::default();
    let a = numeric(&mut e, "x = real(interpft([1 1i -1 -1i], 8));");
    for (i, &v) in a.iter().enumerate() {
        assert!((v - (i as f64 * std::f64::consts::PI / 4.).cos()).abs() < 1e-14);
    }
    assert_eq!(
        numeric(&mut e, "x = interp1q([1;2;4], [2;4;8], [1;4]);"),
        vec![2., 8.]
    );
    assert!(e.execute("interpft([1 2], 1000000000)").is_err());
    assert!(e.execute("peaks(1000000)").is_err());
    e.cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        e.execute("peaks(3)")
            .unwrap_err()
            .to_lowercase()
            .contains("cancel")
    );
}

/// `[b, n] = bucky; [i, j] = find(b)` as in matGeom createSoccerBall.m. The
/// expected structure is the public truncated icosahedron (60 vertices, 90
/// equal edges, three per vertex, all on one sphere); no vertex numbering,
/// orientation or coordinate value is assumed. Distances are computed here,
/// not by the engine, and compared with tolerances.
#[test]
fn bucky_two_outputs_are_one_soccer_ball_graph_on_its_own_vertices() {
    let mut e = Engine::default();
    e.execute("[b, n] = bucky; [i, j] = find(b); e = unique(sort([i j], 2), 'rows');")
        .unwrap();
    assert_eq!(
        numeric(
            &mut e,
            "x = [issparse(b) isa(b, 'double') islogical(b) size(b) size(n) size([i j]) size(e)];"
        ),
        vec![1., 1., 0., 60., 60., 60., 3., 180., 2., 90., 2.]
    );
    let adjacency = numeric(&mut e, "x = full(b);");
    let at = |r: usize, c: usize| adjacency[r + 60 * c];
    for r in 0..60 {
        assert_eq!(at(r, r), 0., "self-loop at {r}");
        for c in 0..60 {
            assert!(at(r, c) == 0. || at(r, c) == 1., "({r},{c}) = {}", at(r, c));
            assert_eq!(at(r, c), at(c, r), "asymmetric at ({r},{c})");
        }
        assert_eq!((0..60).filter(|&c| at(r, c) == 1.).count(), 3, "row {r}");
    }
    // find: 180 directed pairs, each an adjacency entry; 90 undirected edges.
    let pairs = numeric(&mut e, "x = [i j];");
    let mut undirected = std::collections::BTreeSet::new();
    for k in 0..180 {
        let (r, c) = (pairs[k] as usize - 1, pairs[180 + k] as usize - 1);
        assert_eq!(at(r, c), 1., "find pair {k}");
        undirected.insert((r.min(c), r.max(c)));
    }
    assert_eq!(undirected.len(), 90);
    let edges = numeric(&mut e, "x = e;");
    let edges: std::collections::BTreeSet<_> = (0..90)
        .map(|k| (edges[k] as usize - 1, edges[90 + k] as usize - 1))
        .collect();
    assert_eq!(edges, undirected);

    // Coordinates: second output, 60 finite distinct points on one sphere.
    let n = numeric(&mut e, "x = n;");
    assert!(n.iter().all(|v| v.is_finite()));
    let centroid: [f64; 3] =
        std::array::from_fn(|k| n[60 * k..60 * (k + 1)].iter().sum::<f64>() / 60.);
    let point = |r: usize| -> [f64; 3] { std::array::from_fn(|k| n[r + 60 * k] - centroid[k]) };
    let distance =
        |p: [f64; 3], q: [f64; 3]| (0..3).map(|k| (p[k] - q[k]).powi(2)).sum::<f64>().sqrt();
    let radii: Vec<f64> = (0..60).map(|r| distance(point(r), [0.; 3])).collect();
    let radius = radii[0];
    assert!(radius > 0.1);
    for (r, &v) in radii.iter().enumerate() {
        assert!(
            (v - radius).abs() <= 1e-12 * radius,
            "radius {r}: {v} vs {radius}"
        );
    }
    // Each vertex: three equally nearest neighbours, clearly nearer than the
    // fourth (next distances are the pentagon diagonal φ·edge and the hexagon
    // short diagonal √3·edge), and they are exactly its adjacent vertices.
    let mut edge = None;
    for r in 0..60 {
        let mut near: Vec<(f64, usize)> = (0..60)
            .filter(|&c| c != r)
            .map(|c| (distance(point(r), point(c)), c))
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert!(near[0].0 > 1e-6 * radius, "duplicate vertex {r}");
        let length = *edge.get_or_insert(near[0].0);
        for &(d, _) in &near[..3] {
            assert!(
                (d - length).abs() <= 1e-12 * radius,
                "vertex {r}: {d} vs {length}"
            );
        }
        assert!(
            near[3].0 > 1.5 * length,
            "vertex {r}: fourth at {}",
            near[3].0
        );
        let mut nearest: Vec<usize> = near[..3].iter().map(|&(_, c)| c).collect();
        nearest.sort_unstable();
        let adjacent: Vec<usize> = (0..60).filter(|&c| at(r, c) == 1.).collect();
        assert_eq!(nearest, adjacent, "vertex {r}");
    }
    // Published truncated-icosahedron circumradius R = a √(58 + 18√5) / 4 for
    // edge a; the help page documents the unit sphere about the origin.
    let length = edge.unwrap();
    assert!((radius / length - (58. + 18. * 5f64.sqrt()).sqrt() / 4.).abs() <= 1e-12);
    assert!((radius - 1.).abs() <= 1e-12 && centroid.iter().all(|c| c.abs() <= 1e-12));

    // One-output forms give the same adjacency; outputs are deterministic.
    assert_eq!(
        numeric(
            &mut e,
            "b1 = bucky; b2 = bucky(); [b3] = bucky; [~, n2] = bucky; \
             x = [issparse(b1) isequal(b1, b) isequal(b2, b) isequal(b3, b) isequal(n2, n)];"
        ),
        vec![1.; 5]
    );
    let mut fresh = Engine::default();
    fresh.execute("[b, n] = bucky;").unwrap();
    assert_eq!(numeric(&mut fresh, "x = full(b);"), adjacency);
    assert_eq!(
        numeric(&mut fresh, "x = n;")
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        n.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
    for (source, message) in [
        ("bucky(1)", "Too many input arguments."),
        ("[p, q, r] = bucky;", "Too many output arguments."),
    ] {
        let error = e.execute(source).unwrap_err();
        assert!(error.ends_with(message), "{source}: {error}");
    }
    let mut shadowed = Engine::default();
    shadowed.execute("bucky = 7; x = bucky;").unwrap();
    assert_eq!(shadowed.variables["x"].data(), &[7.]);
}

/// A release-only timing probe, invoked explicitly; no speed assertion.
/// Pools are bounded and reused, and setup is outside the timed execution.
#[test]
#[ignore = "release benchmark; run with --ignored --nocapture"]
fn msc_release_benchmark() {
    use std::time::Instant;
    let workloads = [
        (
            "bits",
            "a = uint64(ones(512,512));",
            "x = bitset(a, 40);",
            8,
        ),
        (
            "bitget",
            "a = uint64(ones(512,512));",
            "x = bitget(a,1);",
            8,
        ),
        ("swapbytes", "a = ones(512,512);", "x = swapbytes(a);", 8),
        ("num2hex", "a = ones(1,4096);", "x = num2hex(a);", 8),
        (
            "hex2num",
            "a = num2hex(ones(1,4096));",
            "x = hex2num(a);",
            8,
        ),
        ("permute", "a = ones(512,512);", "x = permute(a,[2 1]);", 8),
        ("flipdim", "a = ones(512,512);", "x = flipdim(a,1);", 8),
        (
            "filter2",
            "a = ones(256,256); h = ones(5,5);",
            "x = filter2(h,a);",
            5,
        ),
        (
            "interp1q",
            "a = (1:4096)'; b = linspace(1,4096,65536)';",
            "x = interp1q(a,a,b);",
            5,
        ),
        (
            "rectint",
            "a = repmat([0 0 1 1],1024,1);",
            "x = rectint(a,a);",
            5,
        ),
        (
            "polyarea",
            "a = repmat([0;1;1;0],1,65536); b = repmat([0;0;1;1],1,65536);",
            "x = polyarea(a,b);",
            5,
        ),
        (
            "interpft-small",
            "a = [1 2 3 2];",
            "x = interpft(a,8);",
            100,
        ),
        (
            "interpft-large",
            "a = ones(1024,256);",
            "x = interpft(a,2048);",
            5,
        ),
        (
            "inpolygon",
            "a = ones(1,65536)*0.5;",
            "x = inpolygon(a,a,[0 1 1 0],[0 0 1 1]);",
            5,
        ),
        ("peaks", "", "x = peaks(512);", 5),
        (
            "hsv2rgb",
            "a = repmat([0.1 0.2 0.3],65536,1);",
            "x = hsv2rgb(a);",
            5,
        ),
        (
            "rgb2hsv",
            "a = repmat([0.1 0.2 0.3],65536,1);",
            "x = rgb2hsv(a);",
            5,
        ),
        ("atan2d", "a = ones(512,512);", "x = atan2d(a,1);", 5),
        ("full", "a = ones(512,512);", "x = full(a);", 100),
    ];
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (name, setup, source, repetitions) in workloads {
                let mut e = Engine::default();
                e.execute(setup).unwrap();
                e.execute(source).unwrap();
                let start = Instant::now();
                for _ in 0..repetitions {
                    e.execute(source).unwrap();
                }
                println!(
                    "{name}: workers={workers}, repetitions={repetitions}, mean_ms={:.4}",
                    start.elapsed().as_secs_f64() * 1000. / f64::from(repetitions)
                );
            }
        });
    }
}
