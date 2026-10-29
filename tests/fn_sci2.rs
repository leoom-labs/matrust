//! Replays the reference corpus of fftn, ifftn, convn, interp3, interpn,
//! griddata, griddatan, delaunay, delaunayn, convhull, convhulln, dsearchn,
//! tsearchn, voronoin, quad2d, quadl, quadv, freqspace and iqr in a fresh
//! engine per case. Block format of `tests/fixtures/fn/sci2-expected.txt`
//! (cases of `tests/fixtures/fn/sci2-cases.txt`; `LONG` marks `format long`):
//! `@@CASE <id>|<statement>`, the `evalc` console text, `@@CLASS <class>
//! <size>` (absent on error), `@@END`; an error case holds `ERR <message>`.
//!
//! The console text, class and size are compared byte for byte, error
//! messages exactly. KNOWN lists external limitations and the explicitly
//! permitted, measured cubic discrepancy; HARNESS lists every relaxed rule.
use matrust::{Array, Engine};

const CORPUS: &str = include_str!("fixtures/fn/sci2-expected.txt");
const CASES: &str = include_str!("fixtures/fn/sci2-cases.txt");
const TOTAL: usize = 215;

const ND: &str = "three-dimensional data: MatRust has no N-D arrays";
const ND_POINTS: &str = "three-dimensional points (tetrahedra, 3-D hulls or cells): MatRust implements the 2-D forms; the reference's answer is recorded";

/// External limitations and the measured cubic discrepancy permitted in fix round 1.
const KNOWN: &[(&str, &str)] = &[
    ("11", ND),
    ("15", ND),
    ("23", ND),
    ("34", ND),
    ("35", ND),
    ("38", ND),
    ("39", ND),
    ("40", ND),
    ("41", ND),
    ("42", ND),
    ("43", ND),
    ("44", ND),
    ("45", ND),
    ("46", ND),
    ("47", ND),
    ("48", ND),
    ("49", ND),
    ("50", ND),
    ("51", ND),
    ("53", ND),
    ("70", ND),
    (
        "76",
        "griddata 'cubic' is implemented as a C1 Clough-Tocher interpolant with minimum norm network gradients (Nielson; Renka and Cline): [0.841299794 0.699482726] against the reference's [0.842786675 0.703275435] (differences -1.5e-3 and -3.8e-3; a median-direction cross derivative, which is not C1 across triangles, comes to -2.8e-4 and -2.2e-4). The reference's gradient estimate is unknown; these values cover one probe only",
    ),
    ("84", ND_POINTS),
    ("90", ND_POINTS),
    ("100", ND_POINTS),
    ("105", ND_POINTS),
    ("116", ND_POINTS),
    ("117", ND_POINTS),
    ("122", ND_POINTS),
    ("123", ND_POINTS),
    ("125", ND_POINTS),
    ("150", ND_POINTS),
    ("212", ND),
];

/// MatRust prefixes run-time errors with their position.
const LOCATION: &str = "error text exact after removing MatRust's 'Line L, column C: ' prefix";
/// The reference's desktop markup and stack (it names the reference snippet's folder).
const WARNING: &str = "the reference's `[\\x08`/`]\\x08` warning markup and its `> In ...` backtrace lines removed; the warning line and all other text exact";
/// Case 83 uses MatRust's duplicate-point warning; the fixture remains verbatim.
const GRIDDATA_WARNING: &str = "strip oracle warning markup/backtrace and expect MatRust's duplicate-point wording; all result text exact";
/// Qhull (delaunayn) and CGAL (delaunay) emit triangles in their internal order.
const TRIANGLES: &str = "same class and size; the same rows as vertex sets, each row a rotation of the reference's (same orientation), in any row order";

/// quad2d and the adaptive rules of quadl and quadv are written from the published methods
/// (docs/CLEAN_ROOM.md); where a choice is left open (tile order and counting, the exact
/// stopping test, the smallest step) the value is compared with a tolerance.
const QUAD2D_DIGITS: &str = "last-digit rounding of quad2d's tile sums: same class and size, the value within relative 1e-13";
const QUAD2D_RESIDUE: &str = "quad2d stops at its tolerance and the reference's value carries its own residue of 1e-13 on a polynomial integrand: same class and size, the value within relative 1e-11";
const QUAD2D_RELTOL: &str = "quad2d stops when its error estimate meets RelTol 1e-6 and the two stopping tests do not agree: same class and size, the value within relative 2e-6 of the reference's";
const QUAD2D_BOUND: &str = "quad2d's error bound of a polynomial integrand is a sum of rounding-level differences: same class and size, between 0 and 1e-12 (the reference's is 2.190164598727010e-13)";
const QUAD2D_LIMIT: &str = "quad2d stopped by MaxFunEvals: the tiles done by then differ from the reference's (MaxFunEvals counts 15 by 15 tile rules here); same class and size, no warning (as in the reference), the value within relative 1e-4 (the reference's own error: it is 8.3e-5 from the exact integral, MatRust's 1e-15)";
/// The stopping test of the adaptive rules is not fixed by the published method; the call's
/// tolerance is what both answers must meet.
const ADAPTIVE: &str = "adaptive stopping test: same class and size, the value within the call's tolerance of the reference's (relative; 1e-6 by default)";
/// A singular integrand (1/t) is cut off at a minimum step that is not published.
const SINGULAR: &str = "singular integrand cut off at a minimum step: the reference's warning line exact, same class and size, the value within relative 5e-3 (quadl, whose cut-off moves in steps of ln 11) or 1e-4 (quadv) of the reference's";

/// Every relaxation of the verbatim comparison.
const HARNESS: &[(&str, &str)] = &[
    ("12", LOCATION),
    ("13", LOCATION),
    ("24", LOCATION),
    ("25", LOCATION),
    ("36", LOCATION),
    ("37", LOCATION),
    ("52", LOCATION),
    ("71", LOCATION),
    ("72", LOCATION),
    ("83", GRIDDATA_WARNING),
    ("85", LOCATION),
    ("86", LOCATION),
    ("91", LOCATION),
    ("92", LOCATION),
    ("93", LOCATION),
    ("94", TRIANGLES),
    ("95", TRIANGLES),
    ("96", TRIANGLES),
    ("97", TRIANGLES),
    ("98", TRIANGLES),
    ("99", LOCATION),
    ("101", LOCATION),
    ("102", LOCATION),
    ("103", TRIANGLES),
    ("106", TRIANGLES),
    ("107", TRIANGLES),
    ("109", LOCATION),
    ("118", LOCATION),
    ("119", LOCATION),
    ("126", LOCATION),
    ("133", LOCATION),
    ("134", LOCATION),
    ("140", LOCATION),
    ("141", LOCATION),
    ("151", LOCATION),
    ("152", LOCATION),
    ("154", QUAD2D_RESIDUE),
    ("155", QUAD2D_RESIDUE),
    ("156", QUAD2D_DIGITS),
    ("157", QUAD2D_BOUND),
    ("158", QUAD2D_RELTOL),
    ("159", QUAD2D_RELTOL),
    ("160", QUAD2D_LIMIT),
    ("161", QUAD2D_DIGITS),
    ("162", LOCATION),
    ("163", LOCATION),
    ("166", ADAPTIVE),
    ("170", WARNING),
    ("171", SINGULAR),
    ("173", LOCATION),
    ("174", LOCATION),
    ("176", ADAPTIVE),
    ("178", ADAPTIVE),
    ("179", ADAPTIVE),
    ("182", SINGULAR),
    ("183", LOCATION),
    ("199", LOCATION),
    ("213", LOCATION),
];

struct Case {
    id: String,
    statement: String,
    long: bool,
    expected: String,
    class: Option<(String, Vec<usize>)>,
}

fn cases() -> Vec<Case> {
    let statements: Vec<&str> = CASES
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .collect();
    let mut cases = Vec::new();
    let mut rest = CORPUS;
    while let Some(start) = rest.find("@@CASE ") {
        rest = &rest[start + "@@CASE ".len()..];
        let (header, after) = rest.split_once('\n').expect("case header line");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (body, after) = after.split_once("@@END\n").expect("case end marker");
        let (expected, class) = match body.split_once("@@CLASS ") {
            Some((text, class)) => {
                let (name, size) = class.trim_end().split_once(' ').expect("class size");
                let size = size
                    .trim_matches(['[', ']'])
                    .split_whitespace()
                    .map(|n| n.parse().expect("size element"))
                    .collect();
                (text.to_string(), Some((name.to_string(), size)))
            }
            None => (body.to_string(), None),
        };
        let source = statements[id.parse::<usize>().expect("numeric id") - 1];
        cases.push(Case {
            id: id.into(),
            statement: statement.into(),
            long: source.starts_with("LONG "),
            expected,
            class,
        });
        rest = after;
    }
    cases
}

/// Drops the reference's `[\x08` `]\x08 ` warning markup and the `> In` stack lines.
fn plain_warnings(text: &str) -> String {
    let text = text.replace("]\u{8} ", "").replace("[\u{8}", "");
    text.lines()
        .filter(|line| !line.starts_with("> In ") && !line.starts_with("  In "))
        .map(|line| format!("{line}\n"))
        .collect()
}

fn unplaced(error: &str) -> &str {
    match error.split_once(": ") {
        Some((place, text)) if place.starts_with("Line ") && place.contains(", column ") => text,
        _ => error,
    }
}

/// The integer rows of a displayed `x = ...` matrix.
fn displayed_rows(text: &str) -> Vec<Vec<i64>> {
    let body = text.split_once("x =").expect("x = display").1;
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            l.split_whitespace()
                .map(|t| t.parse().expect("integer entry"))
                .collect()
        })
        .collect()
}

fn canonical(row: &[i64]) -> Vec<i64> {
    let k = (0..row.len()).min_by_key(|k| row[*k]).unwrap_or(0);
    let mut r = row.to_vec();
    r.rotate_left(k);
    r
}

fn same_triangles(engine: &Engine, expected: &str) -> Option<String> {
    let Some(Array::Numeric(x)) = engine.array("x") else {
        return Some("x is not a double matrix".into());
    };
    if x.data()
        .iter()
        .any(|v| !v.is_finite() || v.fract() != 0. || *v < 1. || *v >= i64::MAX as f64)
    {
        return Some("triangle indices must be finite positive integers".into());
    }
    let mut actual: Vec<Vec<i64>> = (0..x.rows())
        .map(|r| {
            (0..x.cols())
                .map(|c| x.data()[c * x.rows() + r] as i64)
                .collect()
        })
        .collect();
    let mut wanted = displayed_rows(expected);
    if x.cols() == 2 {
        // Segments of a 1-D triangulation have no orientation.
        for r in actual.iter_mut().chain(wanted.iter_mut()) {
            r.sort_unstable();
        }
    }
    let mut a: Vec<Vec<i64>> = actual.iter().map(|r| canonical(r)).collect();
    let mut w: Vec<Vec<i64>> = wanted.iter().map(|r| canonical(r)).collect();
    a.sort();
    w.sort();
    (a != w).then(|| format!("rows {actual:?}, expected the set {wanted:?}"))
}

/// `actual` equals `wanted` within a relative tolerance. NaN matches only a
/// NaN answer and an infinity only itself (a NaN difference is never "small").
fn within(actual: f64, wanted: f64, tolerance: f64) -> bool {
    (actual.is_nan() && wanted.is_nan())
        || actual == wanted
        || (actual.is_finite()
            && wanted.is_finite()
            && (actual - wanted).abs() <= tolerance * wanted.abs())
}

/// Relative tolerance of the singular cases. quadl (171): the cut-off can only move by a
/// factor of 11 (ln 11 = 2.4 in the value), the nearest one leaves 0.18 (4.1e-3) in the
/// rule on the last interval. quadv (182): its first part is chosen so that the cut-off
/// falls where the reference's does; 3.5e-5 is left.
fn singular_tolerance(id: &str) -> f64 {
    if id == "171" { 5e-3 } else { 1e-4 }
}

/// Relative tolerance of the adaptive-rule cases: the tolerance the statement asks for.
fn adaptive_tolerance(id: &str) -> f64 {
    match id {
        "166" => 1e-11,
        "179" => 1e-10,
        _ => 1e-6,
    }
}

fn mismatch(case: &Case) -> Option<String> {
    let mut engine = Engine::default();
    if case.long {
        engine.execute("format long").expect("format long");
    }
    let result = engine.execute(&case.statement);
    let rule = HARNESS
        .iter()
        .find(|(id, _)| *id == case.id)
        .map(|(_, rule)| *rule);
    if let Some(message) = case.expected.strip_prefix("ERR ") {
        let message = message.strip_suffix('\n').expect("error line ending");
        return match result {
            Err(e) => {
                let e = if rule == Some(LOCATION) {
                    unplaced(&e)
                } else {
                    &e
                };
                (e != message).then(|| format!("expected error {message:?}, got error {e:?}"))
            }
            Ok(_) => Some(format!("expected error {message:?}, got success")),
        };
    }
    let actual: String = match result {
        Ok(result) => result.output.iter().map(|e| format!("{e}\n")).collect(),
        Err(e) => return Some(format!("unexpected error {e:?}")),
    };
    let expected = if rule == Some(GRIDDATA_WARNING) {
        plain_warnings(&case.expected).replace(
            "Warning: Duplicate data points have been detected and removed - corresponding values have been averaged.",
            "Warning: Merged duplicate data points and averaged their values.",
        )
    } else if rule == Some(WARNING) || rule == Some(SINGULAR) {
        plain_warnings(&case.expected)
    } else {
        case.expected.clone()
    };
    let tolerance = match rule {
        Some(QUAD2D_DIGITS) => Some(1e-13),
        Some(QUAD2D_RESIDUE) => Some(1e-11),
        Some(QUAD2D_RELTOL) => Some(2e-6),
        Some(QUAD2D_LIMIT) => Some(1e-4),
        Some(ADAPTIVE) => Some(adaptive_tolerance(&case.id)),
        Some(SINGULAR) => Some(singular_tolerance(&case.id)),
        _ => None,
    };
    if rule == Some(TRIANGLES) {
        if let Some(why) = same_triangles(&engine, &expected) {
            return Some(why);
        }
    } else if rule == Some(QUAD2D_BOUND) {
        if actual.contains("Warning") {
            return Some(format!("unexpected warning {actual:?}"));
        }
        let Some(Array::Numeric(x)) = engine.array("x") else {
            return Some("x is not a double matrix".into());
        };
        if x.data().len() != 1 || !(0. ..=1e-12).contains(&x.data()[0]) {
            return Some(format!(
                "values {:?}, expected between 0 and 1e-12",
                x.data()
            ));
        }
    } else if let Some(tolerance) = tolerance {
        let lead = |text: &str| {
            text.split_once("x =")
                .map_or(text, |(lead, _)| lead)
                .to_string()
        };
        if rule == Some(SINGULAR) {
            if lead(&actual) != lead(&expected) {
                return Some(format!(
                    "expected the text {:?} before the value, got {:?}",
                    lead(&expected),
                    lead(&actual)
                ));
            }
        } else if actual.contains("Warning") {
            return Some(format!("unexpected warning {actual:?}"));
        }
        let wanted: Vec<f64> = expected
            .split_once("x =")
            .expect("x = display")
            .1
            .split_whitespace()
            .map(|t| t.parse().expect("number"))
            .collect();
        let Some(Array::Numeric(x)) = engine.array("x") else {
            return Some("x is not a double matrix".into());
        };
        if x.data().len() != wanted.len()
            || x.data()
                .iter()
                .zip(&wanted)
                .any(|(a, w)| !within(*a, *w, tolerance))
        {
            return Some(format!("values {:?}, expected {wanted:?}", x.data()));
        }
    } else if actual != expected {
        return Some(format!("expected {expected:?}\n  actual   {actual:?}"));
    }
    if let Some((class, size)) = &case.class {
        let Some(x) = engine.array("x") else {
            return Some("no variable x".into());
        };
        let got = (x.class().to_string(), vec![x.rows(), x.cols()]);
        if got != (class.clone(), size.clone()) {
            return Some(format!("class/size expected {class} {size:?}, got {got:?}"));
        }
    }
    None
}

#[test]
fn corpus_has_every_case() {
    let all = cases();
    assert_eq!(all.len(), TOTAL);
    let statements: Vec<&str> = CASES
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .collect();
    assert_eq!(statements.len(), TOTAL);
    for (case, source) in all.iter().zip(&statements) {
        assert_eq!(
            case.statement,
            source.strip_prefix("LONG ").unwrap_or(source)
        );
    }
    assert_eq!(
        all.iter()
            .map(|c| c.id.parse::<usize>().unwrap())
            .collect::<Vec<_>>(),
        (1..=TOTAL).collect::<Vec<_>>()
    );
    let mut listed = std::collections::BTreeSet::new();
    for (id, reason) in KNOWN.iter().chain(HARNESS) {
        assert!(!reason.is_empty());
        assert!(listed.insert(id), "case {id} is listed more than once");
        assert!(
            all.iter().any(|c| c.id == *id),
            "listed case {id} is in the corpus"
        );
    }
    for (id, rule) in HARNESS {
        let case = all.iter().find(|c| c.id == *id).unwrap();
        if *rule == LOCATION {
            assert!(case.expected.starts_with("ERR "), "LOCATION case {id}");
        }
    }
}

#[test]
fn matches_reference_corpus() {
    let mut failures = Vec::new();
    let mut passed = 0;
    for case in cases() {
        let known = KNOWN.iter().any(|(id, _)| *id == case.id);
        match (mismatch(&case), known) {
            (None, false) => passed += 1,
            (None, true) => failures.push(format!("case {} passes; remove it from KNOWN", case.id)),
            (Some(_), true) => {}
            (Some(why), false) => {
                failures.push(format!("case {} `{}`\n  {why}", case.id, case.statement))
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{passed} passed, {} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(passed + KNOWN.len(), TOTAL);
    println!("{passed}/{TOTAL} pass, {} KNOWN", KNOWN.len());
}

fn values(engine: &mut Engine, source: &str) -> Vec<f64> {
    engine.execute(source).expect(source);
    match engine.array("x").expect("x") {
        Array::Numeric(m) => m.data().to_vec(),
        a => panic!("unexpected {} result", a.class()),
    }
}

/// Rev3: nearly co-circular sites must not create an overlapping cavity.
#[test]
fn rotated_grid_triangulation_has_positive_areas_and_covers_hull() {
    let mut e = Engine::default();
    let triangles = values(
        &mut e,
        "[X,Y]=meshgrid(0:0.1:2); c=cos(0.3); s=sin(0.3); \
         P=[c*X(:)-s*Y(:) s*X(:)+c*Y(:)]; x=delaunay(P);",
    );
    let Array::Numeric(points) = e.array("P").unwrap() else {
        panic!("numeric points")
    };
    let count = triangles.len() / 3;
    assert!(count >= 800, "{count} triangles");
    let mut total = 0.;
    for i in 0..count {
        let vertex = |k| {
            let index = triangles[i + k * count] as usize - 1;
            robust::Coord {
                x: points.data()[index],
                y: points.data()[index + points.rows()],
            }
        };
        // Rotation rounds almost-collinear boundary sites onto opposite sides
        // of their ideal edge. Measure these very thin triangles adaptively:
        // a plain floating-point determinant can round a positive area to zero.
        let area = robust::orient2d(vertex(0), vertex(1), vertex(2)) / 2.;
        assert!(area > 0., "triangle {i}: signed area {area}");
        total += area;
    }
    assert!((total - 4.).abs() <= 4e-12, "area {total}");
}

/// Rev3: check all 900 interior queries, so a NaN cannot hide in max().
#[test]
fn rotated_griddata_reproduces_planes_at_every_interior_query() {
    let mut e = Engine::default();
    e.execute(
        "[X,Y]=meshgrid(0:0.1:3); c=cos(0.3); s=sin(0.3); \
         px=c*X(:)-s*Y(:); py=s*X(:)+c*Y(:); v=2*px+3*py; \
         [U,W]=meshgrid(0.05:0.1:2.95); qx=c*U-s*W; qy=s*U+c*W;",
    )
    .unwrap();
    for method in ["linear", "cubic", "natural"] {
        let errors = values(
            &mut e,
            &format!("x=griddata(px,py,v,qx,qy,'{method}')-(2*qx+3*qy);"),
        );
        assert_eq!(errors.len(), 900, "{method}");
        for (i, error) in errors.iter().enumerate() {
            assert!(error.abs() <= 1e-9, "{method} query {i}: error {error}");
        }
    }
}

/// Rev3: all griddata methods reject nonfinite sites like delaunay does.
#[test]
fn griddata_rejects_nonfinite_data_coordinates() {
    for bad in ["NaN", "Inf", "-Inf"] {
        for (px, py) in [
            (format!("[0 {bad} 1 0 1]"), "[0 0 0 1 1]".to_owned()),
            ("[0 0 1 0 1]".to_owned(), format!("[0 {bad} 0 1 1]")),
        ] {
            for method in [
                "",
                ",'linear'",
                ",'nearest'",
                ",'natural'",
                ",'cubic'",
                ",'v4'",
            ] {
                let source = format!("griddata({px},{py},[1 2 3 4 5],0.5,0.5{method})");
                let error = refused(&source);
                assert!(
                    error.ends_with("Data points must be finite."),
                    "{source}: {error}"
                );
            }
        }
    }
}

/// Rev3: the reference extrapolates spline by default; explicit fills take precedence.
#[test]
fn interpn_spline_extrapolates_only_without_an_explicit_fill() {
    let mut e = Engine::default();
    let x = values(&mut e, "x=interpn(1:4,[1 4 9 16],5,'spline');");
    assert_eq!(x.len(), 1);
    assert!((x[0] - 25.).abs() < 1e-12, "{x:?}");
    for method in ["linear", "nearest", "pchip", "cubic"] {
        let x = values(
            &mut e,
            &format!("x=interpn(1:4,[1 4 9 16],[0 5],'{method}');"),
        );
        assert_eq!(x.len(), 2);
        assert!(x.iter().all(|v| v.is_nan()), "{method}: {x:?}");
    }
    let x = values(&mut e, "x=interpn(1:4,[1 4 9 16],[0 2.5 5],'spline',-7);");
    assert_eq!(x[0], -7.);
    assert!((x[1] - 6.25).abs() < 1e-12, "{x:?}");
    assert_eq!(x[2], -7.);
    let x = values(&mut e, "x=interpn(1:4,[1 4 9 16],[0 5 NaN],'spline',NaN);");
    assert!(x.iter().all(|v| v.is_nan()), "{x:?}");
    // Polynomial reproduction on both ends and along either tensor dimension.
    let x = values(
        &mut e,
        "[X,Y]=ndgrid(1:4); V=X.^2+Y.^2; \
         x=interpn(X,Y,V,[0 2 5 0],[2 0 2 5],'spline')-[4 4 29 25];",
    );
    assert!(x.iter().all(|v| v.abs() < 1e-12), "{x:?}");
}

#[test]
fn griddata_rejects_arguments_after_the_method() {
    for method in ["linear", "nearest", "natural", "cubic", "v4"] {
        for extra in ["'junk'", "7", "'junk',7"] {
            let source = format!("griddata([0 1 0],[0 0 1],[1 2 3],0.25,0.25,'{method}',{extra})");
            let error = refused(&source);
            assert!(
                error.ends_with("Too many input arguments."),
                "{source}: {error}"
            );
        }
    }
}

#[test]
fn delaunayn_rejects_nonfinite_one_dimensional_sites() {
    for bad in ["NaN", "Inf", "-Inf"] {
        for points in [format!("[1;{bad};3]"), format!("[{bad};1;3]")] {
            let source = format!("delaunayn({points})");
            let error = refused(&source);
            assert!(
                error.ends_with("Data points must be finite."),
                "{source}: {error}"
            );
        }
    }
    assert_eq!(
        values(&mut Engine::default(), "x=delaunayn([3;1;2]);"),
        [2., 3., 3., 1.]
    );
}

#[test]
fn interpn_distinguishes_refinement_limits_from_invalid_ntimes() {
    for times in ["17", "1000000", "1e100"] {
        let source = format!("interpn([1 2],{times})");
        let error = refused(&source);
        assert!(error.contains("resource limit"), "{source}: {error}");
        assert!(!error.contains("must be a nonnegative integer"), "{error}");
    }
    for times in ["-1", "0.5", "NaN", "Inf", "-Inf"] {
        let source = format!("interpn([1 2],{times})");
        let error = refused(&source);
        assert!(
            error.ends_with("NTIMES must be a nonnegative integer."),
            "{source}: {error}"
        );
    }
    assert_eq!(
        values(&mut Engine::default(), "x=interpn([1 2],0);"),
        [1., 2.]
    );
    assert_eq!(
        values(&mut Engine::default(), "x=interpn([1 2],1);"),
        [1., 1.5, 2.]
    );
}

/// Not reference-checked: grids are co-circular everywhere; the triangulation
/// must still cover the hull once (Euler: 2n - 2 - h triangles), with every
/// triangle counter-clockwise.
#[test]
fn degenerate_grids_triangulate_consistently() {
    let mut e = Engine::default();
    for (grid, n, h) in [("0:4", 25f64, 16.), ("0:0.5:3", 49., 24.)] {
        let x = values(
            &mut e,
            &format!(
                "[X, Y] = meshgrid({grid}); P = [X(:) Y(:)]; T = delaunay(P); \
                 A = (P(T(:,2),1)-P(T(:,1),1)).*(P(T(:,3),2)-P(T(:,1),2)) - \
                 (P(T(:,3),1)-P(T(:,1),1)).*(P(T(:,2),2)-P(T(:,1),2)); \
                 x = [size(T, 1), min(A), sum(A)/2];"
            ),
        );
        let side = n.sqrt() - 1.;
        let step = if grid == "0:4" { 1. } else { 0.5 };
        assert_eq!(x[0], 2. * n - 2. - h, "{grid}");
        assert!(x[1] > 0., "{grid}: a triangle is not counter-clockwise");
        assert!(
            (x[2] - (side * step).powi(2)).abs() < 1e-12,
            "{grid}: area {}",
            x[2]
        );
    }
    // Duplicates are used once; the hull keeps its collinear points.
    let x = values(
        &mut e,
        "x = size(delaunay([0 1 2 0 1 2 0], [0 0 0 1 1 1 0]));",
    );
    assert_eq!(x, [4., 3.]);
    let x = values(
        &mut e,
        "[k, a] = convhull([0 1 2 2 1 0], [0 0 0 1 1 1]); x = [k' a];",
    );
    assert_eq!(x, [1., 2., 3., 4., 5., 6., 1., 2.]);
}

/// Not reference-checked: natural-neighbour interpolation reproduces linear
/// data exactly, also on the shared edges of a grid (where the circumcentre
/// of the query point and an interior edge is at infinity).
#[test]
fn natural_neighbours_reproduce_linear_data_on_grid_edges() {
    let mut e = Engine::default();
    let x = values(
        &mut e,
        "[X, Y] = meshgrid(0:0.25:1); V = 2*X - Y + 1; \
         q = [0.3 0.2; 0.5 0.5; 0.375 0.125; 0.6 0.9; 0.25 0.6]; \
         x = griddata(X(:), Y(:), V(:), q(:,1), q(:,2), 'natural')' - (2*q(:,1) - q(:,2) + 1)';",
    );
    assert!(x.iter().all(|v| v.abs() < 1e-12), "{x:?}");
    let x = values(
        &mut e,
        "[X, Y] = meshgrid(0:3); V = X + 10*Y; x = griddata(X(:), Y(:), V(:), [1.5 2.5 4], [0.5 2.5 1]);",
    );
    assert_eq!(&x[..2], &[6.5, 27.5]);
    assert!(x[2].is_nan());
}

/// Not reference-checked: dimensions longer than 4 go through the engine's fft
/// and agree with fft2; the inverse returns the data.
#[test]
fn long_dimensions_agree_with_fft2() {
    let mut e = Engine::default();
    let x = values(
        &mut e,
        "A = magic(6); A(2, :) = []; B = fftn(A); C = fft2(A); \
         x = [max(abs(B(:) - C(:))), max(max(abs(ifftn(B) - A))), isreal(ifftn(B))];",
    );
    assert!(x[0] < 1e-12 && x[1] < 1e-12, "{x:?}");
    assert_eq!(x[2], 1.);
    let x = values(&mut e, "x = size(fftn(ones(3, 7), [5 2]));");
    assert_eq!(x, [5., 2.]);
}

/// Not reference-checked: interpn agrees with interp2 on transposed (ndgrid
/// versus meshgrid) layouts, and refines a matrix grid.
#[test]
fn interpn_matches_interp2_and_refines() {
    let mut e = Engine::default();
    let x = values(
        &mut e,
        "V = magic(4); a = interpn(V, [1.5 3.25], [2.5 1.75]); \
         b = interp2(V, [2.5 1.75], [1.5 3.25]); x = a - b;",
    );
    assert!(x.iter().all(|v| v.abs() < 1e-12), "{x:?}");
    let x = values(&mut e, "x = size(interpn(magic(3), 2));");
    assert_eq!(x, [9., 9.]);
}

/// Not reference-checked: oversized searches are refused before they start.
#[test]
fn searches_are_bounded() {
    let mut e = Engine::default();
    let error = e
        .execute("dsearchn(zeros(9000, 2), zeros(9000, 2))")
        .unwrap_err();
    assert!(error.contains("point pairs"), "{error}");
    let error = e
        .execute("delaunay(zeros(300000, 1), zeros(300000, 1))")
        .unwrap_err();
    assert!(error.contains("200000"), "{error}");
}

fn refused(source: &str) -> String {
    Engine::default()
        .execute(source)
        .expect_err(&format!("`{source}` should be refused"))
}

/// Review findings 1-4: the element limit and the evaluation budget are
/// checked before the request allocates (each of these once asked for
/// gigabytes before any check).
#[test]
fn oversized_requests_are_refused_before_allocation() {
    for (source, expected) in [
        // 65537 x 65537 refined query points.
        ("interpn(ones(2), 16)", "element limit"),
        ("interpn(ones(2), 1:5000, (1:5000)')", "element limit"),
        (
            "griddata([0 1 0], [0 0 1], [1 2 3], 1:100000, (1:100000)')",
            "element limit",
        ),
        ("[a, b] = freqspace(1e12)", "element limit"),
        ("[a, b] = freqspace(1e12, 'meshgrid')", "element limit"),
        ("freqspace([5000 5000], 'meshgrid')", "element limit"),
        ("x = freqspace(1e12)", "element limit"),
        ("x = freqspace(1e100)", "element limit"),
        ("x = freqspace(1e100, 'whole')", "element limit"),
        (
            "tsearchn(zeros(1, 100000), ones(1, 100001), zeros(1, 100000))",
            "too large",
        ),
    ] {
        let error = refused(source);
        assert!(error.contains(expected), "`{source}`: {error}");
    }
}

/// Review finding 9: freqspace charges the vectors it returns, not their
/// product.
#[test]
fn freqspace_charges_only_what_it_returns() {
    let mut e = Engine::default();
    let x = values(
        &mut e,
        "[a, b] = freqspace(10000); x = [size(a) size(b) a(1) a(end)];",
    );
    assert_eq!(x, [1., 10000., 1., 10000., -1., 0.9998]);
    let x = values(
        &mut e,
        "[a, b] = freqspace([4 6], 'meshgrid'); x = [size(a) a(1, :) b(:, 1)'];",
    );
    assert_eq!(
        x,
        [
            4.,
            6.,
            -1.,
            -2. / 3.,
            -1. / 3.,
            0.,
            1. / 3.,
            2. / 3.,
            -1.,
            -0.5,
            0.,
            0.5
        ]
    );
    let x = values(&mut e, "x = freqspace([4 6], 'meshgrid') - a;");
    assert!(x.iter().all(|v| *v == 0.));
}

/// Review finding 5: an empty grid is an error, not an out-of-range index.
#[test]
fn interpn_rejects_empty_grids() {
    for source in [
        "interpn([], 1, 1)",
        "interpn([])",
        "interpn([], 2)",
        "interpn([], [], [], [], [])",
        "interpn(zeros(0, 3), [1 2], [1 2])",
        "interpn(zeros(3, 0), 1, 1)",
        "interpn(zeros(3, 0), zeros(0, 3), [], 1, 1)",
        "interpn(zeros(3, 0), 1:3, ones(3), 1, 1)",
        "interpn(1:3, zeros(0, 3), ones(3), 1, 1)",
    ] {
        let error = refused(source);
        assert!(error.contains("must not be empty"), "`{source}`: {error}");
    }
}

/// Review finding 7: a site is on the hull exactly when its triangle fan does
/// not close; the interior point's skinny triangles span more than pi.
#[test]
fn voronoi_interior_cell_is_bounded() {
    let mut e = Engine::default();
    let x = values(
        &mut e,
        "[V, C] = voronoin([-1 0; 1 0; 0 100; 0.9 0.1]); \
         x = [any(C{1} == 1) any(C{2} == 1) any(C{3} == 1) any(C{4} == 1) numel(C{4})];",
    );
    assert_eq!(x, [1., 1., 1., 0., 3.]);
}

/// Review finding 8: 'symmetric' reads the first half of the spectrum and
/// rebuilds the conjugate half; the second half of the input is ignored.
#[test]
fn ifftn_symmetric_rebuilds_the_conjugate_half() {
    let mut e = Engine::default();
    let x = values(&mut e, "x = ifftn([0 1 0 0], 'symmetric');");
    assert_eq!(x, [0.5, 0., -0.5, 0.]);
    // Longer vector (engine ifft path) and a column.
    let x = values(
        &mut e,
        "A = [1 5 2 8 3 7 4 6]; B = fft(A); B(6:8) = 99; \
         x = [max(abs(ifftn(B, 'symmetric') - A)) max(abs(ifftn(B.', 'symmetric') - A.'))];",
    );
    assert!(x.iter().all(|v| *v < 1e-12), "{x:?}");
    // Small matrix (FFTW-style kernel) and larger matrix: rows past the half
    // are rebuilt from the first half.
    let x = values(
        &mut e,
        "A = magic(4); B = fftn(A); B(4, :) = 7; \
         x = max(max(abs(ifftn(B, 'symmetric') - A)));",
    );
    assert!(x[0] < 1e-12, "{x:?}");
    let x = values(
        &mut e,
        "A = magic(6); A(2, :) = []; B = fftn(A); B(4:5, :) = 99; \
         x = [max(max(abs(ifftn(B, 'symmetric') - A))) isreal(ifftn(B, 'symmetric'))];",
    );
    assert!(x[0] < 1e-12 && x[1] == 1., "{x:?}");
    // Without the flag the data are used as given.
    let x = values(&mut e, "x = double(isreal(ifftn([0 1 0 0])));");
    assert_eq!(x, [0.]);
}

/// Review finding 10: a relaxed comparison rejects NaN unless NaN is expected.
#[test]
fn relaxed_comparison_rejects_nan() {
    // The four relaxed quad2d cases must reject a NaN actual answer.
    for id in ["156", "157", "160", "161"] {
        let mut case = cases().into_iter().find(|c| c.id == id).unwrap();
        case.statement = "x = NaN".into();
        assert!(mismatch(&case).unwrap().starts_with("values"), "case {id}");
    }
    for wanted in [1., f64::INFINITY, f64::NEG_INFINITY] {
        assert!(!within(f64::NAN, wanted, 1e-5));
        assert!(!within(wanted, f64::NAN, 1e-5));
    }
    for infinity in [f64::INFINITY, f64::NEG_INFINITY] {
        assert!(!within(infinity, 1., 1e-5));
        assert!(!within(1., infinity, 1e-5));
        assert!(!within(-infinity, infinity, 1e-5));
        assert!(within(infinity, infinity, 1e-5));
    }
    assert!(!within(1.1, 1., 1e-5));
    assert!(within(f64::NAN, f64::NAN, 1e-5));
    assert!(within(f64::INFINITY, f64::INFINITY, 1e-5));
    assert!(within(1. + 1e-9, 1., 1e-5));
    assert!(within(0., 0., 1e-5));
}

/// Not reference-checked: griddata 'cubic' (Clough-Tocher, minimum norm network
/// gradients) reproduces linear data, is C1 across an interior edge, gives
/// NaN outside the hull, and agrees with the independent Python prototype
/// `scratchpad/sci2/cubic_ref.py` on corpus case 76.
#[test]
fn griddata_cubic_is_a_c1_interpolant() {
    let mut e = Engine::default();
    let x = values(
        &mut e,
        "px = [0 1 1 0 0.5 0.2 0.8]; py = [0 0 1 1 0.5 0.7 0.3]; \
         q = [0.25 0.6 0.9 0.5 0.1; 0.75 0.3 0.1 0.9 0.2]; \
         x = griddata(px, py, 2*px + 3*py + 1, q(1, :), q(2, :), 'cubic') - (2*q(1, :) + 3*q(2, :) + 1);",
    );
    assert!(x.iter().all(|v| v.abs() < 1e-12), "{x:?}");
    // The data points themselves.
    let x = values(
        &mut e,
        "x = griddata(px, py, px.^2 + py, px, py, 'cubic') - (px.^2 + py);",
    );
    assert!(x.iter().all(|v| v.abs() < 1e-12), "{x:?}");
    let x = values(
        &mut e,
        "x = griddata(px, py, px.^2 + py, [2 0.5], [2 0.5], 'cubic');",
    );
    assert!(x[0].is_nan() && x[1].is_finite(), "{x:?}");
    // C1: the normal derivative agrees on both sides of the edge from
    // (0, 0) to (0.5, 0.5) (curved data, so linear interpolation jumps).
    let jump = |e: &mut Engine, method: &str| {
        values(
            e,
            &format!(
                "h = 1e-6; n = [1 -1] / sqrt(2); p = [0.25 0.25]; \
                 f = @(s) griddata(px, py, px.^2 + py, p(1) + s*n(1), p(2) + s*n(2), '{method}'); \
                 x = (f(h) - 2*f(0) + f(-h)) / h;"
            ),
        )[0]
    };
    assert!(
        jump(&mut e, "cubic").abs() < 1e-3,
        "cubic jump {}",
        jump(&mut e, "cubic")
    );
    assert!(
        jump(&mut e, "linear").abs() > 1e-2,
        "linear jump {}",
        jump(&mut e, "linear")
    );
    // Four hundred scattered points: the iterative gradient solve converges
    // (the network reproduces planes, so the answer is exact up to rounding).
    let x = values(
        &mut e,
        "k = (1:400)'; sx = mod(k * 0.6180339887, 1); sy = mod(k * 0.7548776662, 1); \
         [X, Y] = meshgrid(0.3:0.1:0.7); \
         x = max(max(abs(griddata(sx, sy, 2*sx - 3*sy + 1, 0.3:0.1:0.7, (0.3:0.1:0.7)', 'cubic') - (2*X - 3*Y + 1))));",
    );
    assert!(x[0] < 1e-8, "{x:?}");
}

/// Review finding 6: execute the reviewer's corpus example even while it is
/// KNOWN, and retain the measured difference rather than accepting an error.
#[test]
fn griddata_cubic_case_76_measured_difference() {
    let case = cases().into_iter().find(|c| c.id == "76").unwrap();
    let x = values(&mut Engine::default(), &case.statement);
    let wanted: Vec<f64> = case
        .expected
        .split_once("x =")
        .unwrap()
        .1
        .split_whitespace()
        .map(|v| v.parse().unwrap())
        .collect();
    assert_eq!(x.len(), 2);
    assert!((x[0] - 0.8412997941674516).abs() < 1e-9, "{x:?}");
    assert!((x[1] - 0.699482725523754).abs() < 1e-9, "{x:?}");
    let difference: Vec<_> = x.iter().zip(&wanted).map(|(a, w)| a - w).collect();
    assert!((difference[0] + 0.0014868807469864).abs() < 1e-9);
    assert!((difference[1] + 0.003792709497519).abs() < 1e-9);
    println!("case 76: actual {x:?}, reference {wanted:?}, difference {difference:?}");
}

/// Rev2 finding 2: reject the full repeated coefficient work before starting it.
#[test]
fn coefficient_work_is_bounded() {
    for method in ["spline", "pchip"] {
        let source = format!("interpn(ones(1000),1:1000,1:1000,'{method}')");
        assert!(refused(&source).contains("evaluation budget"), "{source}");
    }
}

/// Rev2 finding 3: reduce each empty column and preserve the input precision.
#[test]
fn empty_iqr_preserves_shape_and_single() {
    let mut e = Engine::default();
    for source in ["x=iqr(zeros(0,3));", "x=iqr(zeros(0,3),1);"] {
        assert!(values(&mut e, source).iter().all(|v| v.is_nan()));
        let Array::Numeric(x) = e.array("x").unwrap() else {
            panic!("double result")
        };
        assert_eq!((x.rows(), x.cols()), (1, 3));
    }
    for (input, cols) in [("[]", 1), ("zeros(0,3)", 3)] {
        let x = values(
            &mut e,
            &format!(
                "v=iqr(single({input})); x=[double(strcmp(class(v),'single')) size(v) double(v)];"
            ),
        );
        assert_eq!(&x[..3], &[1., 1., cols as f64]);
        assert_eq!(x.len(), 3 + cols);
        assert!(x[3..].iter().all(|v| v.is_nan()));
    }
}

/// Rev2 finding 4: every full coordinate array must be a matching ndgrid.
#[test]
fn interpn_rejects_malformed_full_grids() {
    for source in [
        "interpn([1 99;2 -5],[1 2;99 -5],[1 2;3 4],1.5,1.5)",
        "interpn([1 1;2 2],[1 2;99 -5],[1 2;3 4],1.5,1.5)",
        "interpn([1 1 1;2 2 2],[1 2;1 2],[1 2;3 4],1.5,1.5)",
    ] {
        let error = refused(source);
        assert!(
            error.contains("NDGRID") || error.contains("same size"),
            "{source}: {error}"
        );
    }
    assert_eq!(
        values(
            &mut Engine::default(),
            "x=interpn([1 1;2 2],[1 2;1 2],[1 2;3 4],1.5,1.5);"
        ),
        [2.5]
    );
}

/// Rev2 finding 5: invalid options fail before the integrand can run.
#[test]
fn quad2d_validates_option_values() {
    for (name, invalid, message) in [
        (
            "AbsTol",
            vec!["-1", "NaN", "Inf", "[1 2]"],
            "AbsTol must be a nonnegative scalar.",
        ),
        (
            "RelTol",
            vec!["-1", "NaN", "Inf", "[1 2]"],
            "RelTol must be a nonnegative scalar.",
        ),
        (
            "MaxFunEvals",
            vec!["0", "-1", "1.5", "NaN", "Inf", "[1 2]"],
            "MaxFunEvals must be a positive integer.",
        ),
        (
            "Singular",
            vec!["-1", "2", "NaN", "Inf", "[1 2]"],
            "Singular must be true or false.",
        ),
        (
            "FailurePlot",
            vec!["-1", "2", "NaN", "Inf", "[1 2]"],
            "FailurePlot must be true or false.",
        ),
    ] {
        for value in invalid {
            let source = format!("quad2d(@(u,v) missing_integrand(u,v),0,1,0,1,'{name}',{value})");
            assert!(refused(&source).ends_with(message), "{source}");
        }
    }
    let x = values(
        &mut Engine::default(),
        "x=quad2d(@(u,v) u+v,0,1,0,1,'AbsTol',0,'RelTol',0,'MaxFunEvals',1,'Singular',false,'FailurePlot',false);",
    );
    assert!((x[0] - 1.).abs() < 1e-12);
}

/// Rev2 finding 6: an unknown flag must never fall through to default output.
#[test]
fn freqspace_rejects_unknown_flags() {
    for source in [
        "freqspace(4,'bogus')",
        "[a,b]=freqspace(4,'bogus')",
        "freqspace(4,3)",
    ] {
        assert!(refused(source).ends_with("FLAG must be 'whole' or 'meshgrid'."));
    }
}

/// Rev2 finding 7: only row ordering and cyclic vertex rotations are relaxed.
#[test]
fn triangle_comparison_requires_integer_indices() {
    let expected = "x =\n 1 2 3\n";
    for actual in [
        "[1.9 2.9 3.9]",
        "[NaN 2 3]",
        "[Inf 2 3]",
        "[0 2 3]",
        "[1 3 2]",
    ] {
        let mut e = Engine::default();
        e.execute(&format!("x={actual};")).unwrap();
        assert!(same_triangles(&e, expected).is_some(), "{actual}");
    }
    let mut e = Engine::default();
    e.execute("x=[2 3 1];").unwrap();
    assert!(same_triangles(&e, expected).is_none());
}

/// Fix round 1: the three-point parabola is a pure quadratic that neither
/// overflows in its end slope nor grows a cubic term from tiny spacings.
#[test]
fn three_point_spline_survives_extreme_magnitudes() {
    let close = |got: f64, want: f64| {
        assert!(((got - want) / want).abs() < 1e-15, "{got:e} vs {want:e}");
    };
    let mut e = Engine::default();
    for call in [
        "spline([0 1 2],[0 1e308 1e308],0.5)",
        "interp1([0 1 2],[0 1e308 1e308],0.5,'spline')",
        "interpn([0 1 2],[0 1e308 1e308],0.5,'spline')",
    ] {
        let x = values(&mut e, &format!("x={call};"));
        close(x[0], 6.25e307);
    }
    for call in [
        "spline([0 1e-201 1e-200],[0 1e-202 1e-200],5e-202)",
        "interp1([0 1e-201 1e-200],[0 1e-202 1e-200],5e-202,'spline')",
        "interpn([0 1e-201 1e-200],[0 1e-202 1e-200],5e-202,'spline')",
    ] {
        let x = values(&mut e, &format!("x={call};"));
        close(x[0], 2.5e-203);
    }
}

/// Fix round 1: argument errors of spline and pchip keep the reference texts
/// (the point count is checked before the clamped-slope case).
#[test]
fn spline_and_pchip_argument_errors_use_the_reference_texts() {
    let few = "There should be at least two data points.";
    let same = "Abscissae and ordinate vectors should be of the same length.";
    for (source, want) in [
        ("spline(1,2,1)", few),
        ("pchip(1,2,1)", few),
        ("spline(0,[1 2 3],0)", few),
        ("spline([1 2 3],[1 2],1.5)", same),
        ("pchip([1 2 3],[1 2],1.5)", same),
        ("spline([1 2],[1 2 3 4 5],1.5)", same),
        ("pchip([1 2],[1 2 3 4],1.5)", same),
    ] {
        let error = refused(source);
        assert!(error.ends_with(want), "{source}: {error}");
    }
}
