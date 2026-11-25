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

const ND: &str = "outside ND_AWARE; expect the centralized size-bearing N-D guard";
const ND_POINTS: &str = "three-dimensional points (tetrahedra or Voronoi cells): MatRust implements the 2-D forms; the reference's answer is recorded";

/// External limitations and the measured cubic discrepancy permitted in fix round 1.
const KNOWN: &[(&str, &str)] = &[
    ("11", ND),
    ("15", ND),
    ("23", ND),
    ("34", ND),
    ("35", ND),
    ("50", "interp3 refinement form is not implemented"),
    ("51", "interp3 refinement form is not implemented"),
    ("70", ND),
    (
        "76",
        "griddata 'cubic' is implemented as a C1 Clough-Tocher interpolant with minimum norm network gradients (Nielson; Renka and Cline): [0.841299794 0.699482726] against the reference's [0.842786675 0.703275435] (differences -1.5e-3 and -3.8e-3; a median-direction cross derivative, which is not C1 across triangles, comes to -2.8e-4 and -2.2e-4). The reference's gradient estimate is unknown; these values cover one probe only",
    ),
    ("84", ND_POINTS),
    ("90", ND_POINTS),
    ("100", ND_POINTS),
    ("105", ND_POINTS),
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

/// convhulln lists Qhull's facets as Qhull stores them (case 122: all four inward); MatRust
/// orients every facet of a 3-D hull outward, as the reference's convhull does (case 116).
const FACETS: &str =
    "same class and size; the same rows as vertex sets, in any vertex and row order";

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
    ("53", LOCATION),
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
    ("122", FACETS),
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
    same_rows(engine, expected, false)
}

/// `unoriented`: the rows are compared as vertex sets.
fn same_rows(engine: &Engine, expected: &str, unoriented: bool) -> Option<String> {
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
    if x.cols() == 2 || unoriented {
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
    if let Some(expected) = nd_guard_expected(&case.id) {
        return match result {
            Err(error) if unplaced(&error) == expected => {
                Some(format!("N-D guard validated: {expected}"))
            }
            Err(error) => Some(format!(
                "N-D guard mismatch: expected {expected:?}, got {error:?}"
            )),
            Ok(_) => Some(format!("N-D guard expected: {expected}")),
        };
    }
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
    if rule == Some(TRIANGLES) || rule == Some(FACETS) {
        if let Some(why) = same_rows(&engine, &expected, rule == Some(FACETS)) {
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

fn nd_guard_expected(id: &str) -> Option<&'static str> {
    Some(match id.parse::<u32>().ok()? {
        11 | 23 => "fftn does not support N-D arrays yet (size 2x2x2)",
        34 | 35 => "convn does not support N-D arrays yet (size 2x2x2)",
        70 => "interpn does not support N-D arrays yet (size 2x2x2)",
        212 => "iqr does not support N-D arrays yet (size 2x2x2)",
        _ => return None,
    })
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
            (Some(why), true)
                if nd_guard_expected(&case.id).is_some()
                    && why.starts_with("N-D guard validated:") => {}
            (Some(why), true) if nd_guard_expected(&case.id).is_some() => {
                failures.push(format!("case {} `{}`\n  {why}", case.id, case.statement));
            }
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
        ("interpn(ones(2), 1:7095, (1:7095)')", "element limit"),
        (
            "griddata([0 1 0], [0 0 1], [1 2 3], 1:100000, (1:100000)')",
            "element limit",
        ),
        ("[a, b] = freqspace(1e12)", "element limit"),
        ("[a, b] = freqspace(1e12, 'meshgrid')", "element limit"),
        ("freqspace([7095 7095], 'meshgrid')", "element limit"),
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

// -- convex hull of three-dimensional points ---------------------------------
//
// Expected vertex sets, triangle counts and volumes: SciPy 1.18.1
// `scipy.spatial.ConvexHull` (Qhull), tool/refs/hull3/convhull3.py, which
// builds every point set with the same double operations as the statements
// here. The triangulation of a flat face and the row order are free.

const HULL_TOO_FEW: &str = "Error computing the convex hull. Not enough unique points specified.";
const HULL_FLAT: &str = "Error computing the convex hull. The points may be coplanar or collinear.";

/// The 8 unit-cube corners (rows 1-8), the centre (9) and the 12 edge midpoints.
const HULL_CUBE: &str = "c = [0 0 0; 0 0 1; 0 1 0; 0 1 1; 1 0 0; 1 0 1; 1 1 0; 1 1 1]; m = zeros(0, 3); \
     for i = 1:8, for j = i+1:8, if sum(abs(c(i,:) - c(j,:))) == 1, \
     m(end+1, :) = (c(i,:) + c(j,:)) / 2; end, end, end, P = [c; 0.5 0.5 0.5; m];";

/// 200 points from the Park-Miller generator (exact in doubles), column-major.
const HULL_SCATTERED: &str = "s = 20261121; r = zeros(600, 1); \
     for k = 1:600, s = mod(16807 * s, 2147483647); r(k) = s / 2147483647; end, \
     P = reshape(r, 200, 3);";

fn matrix_rows(engine: &Engine, name: &str) -> Vec<Vec<f64>> {
    let Some(Array::Numeric(m)) = engine.array(name) else {
        panic!("{name} is not a double matrix");
    };
    (0..m.rows())
        .map(|r| (0..m.cols()).map(|c| m.data()[c * m.rows() + r]).collect())
        .collect()
}

struct Surface {
    /// Sorted 1-based rows of the points used by the triangles.
    vertices: Vec<usize>,
    triangles: usize,
    edges: usize,
    /// By the divergence theorem over the triangles.
    volume: f64,
    smallest_area: f64,
}

/// Checks that the triangles `k` over the points `p` are the surface of the
/// convex hull: every index valid, every edge shared by exactly two triangles
/// that run through it in opposite directions, Euler's formula for a sphere,
/// every triangle with a positive area and its right-hand normal pointing
/// away from the interior, and no point outside the plane of any triangle.
#[allow(clippy::needless_range_loop)]
fn hull_surface(engine: &Engine, p: &str, k: &str) -> Surface {
    let points = matrix_rows(engine, p);
    let rows = matrix_rows(engine, k);
    assert!(rows.len() >= 4, "{} triangles", rows.len());
    let scale = points.iter().flatten().fold(0f64, |m, v| m.max(v.abs()));
    let mut directed = std::collections::BTreeMap::new();
    let mut used = std::collections::BTreeSet::new();
    let mut triangles = Vec::new();
    for row in &rows {
        assert_eq!(row.len(), 3);
        let t: Vec<usize> = row
            .iter()
            .map(|v| {
                assert!(
                    v.fract() == 0. && *v >= 1. && *v <= points.len() as f64,
                    "index {v}"
                );
                *v as usize
            })
            .collect();
        for i in 0..3 {
            *directed.entry((t[i], t[(i + 1) % 3])).or_insert(0) += 1;
            used.insert(t[i]);
        }
        triangles.push(t);
    }
    for ((a, b), count) in &directed {
        assert_eq!(
            *count, 1,
            "edge {a}->{b} is used {count} times in one direction"
        );
        assert!(
            directed.contains_key(&(*b, *a)),
            "edge {a}-{b} has one triangle only"
        );
    }
    let edges = directed.len() / 2;
    assert_eq!(used.len() + rows.len(), edges + 2, "Euler's formula");
    let at = |i: usize| [points[i - 1][0], points[i - 1][1], points[i - 1][2]];
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let mut centre = [0.; 3];
    for v in &used {
        for i in 0..3 {
            centre[i] += at(*v)[i] / used.len() as f64;
        }
    }
    let (mut volume, mut smallest_area) = (0., f64::INFINITY);
    for t in &triangles {
        let (a, b, c) = (at(t[0]), at(t[1]), at(t[2]));
        let normal = cross(sub(b, a), sub(c, a));
        let length = dot(normal, normal).sqrt();
        assert!(length > 0., "triangle {t:?} has no area");
        smallest_area = smallest_area.min(length / 2.);
        assert!(
            dot(normal, sub(centre, a)) < 0.,
            "triangle {t:?} faces inward"
        );
        for (i, q) in points.iter().enumerate() {
            let height = dot(normal, sub([q[0], q[1], q[2]], a)) / length;
            assert!(
                height <= 1e-12 * scale,
                "point {} is {height} outside {t:?}",
                i + 1
            );
        }
        volume += dot(sub(a, centre), cross(sub(b, centre), sub(c, centre))) / 6.;
    }
    Surface {
        vertices: used.into_iter().collect(),
        triangles: rows.len(),
        edges,
        volume,
        smallest_area,
    }
}

fn scalar(engine: &Engine, name: &str) -> f64 {
    let rows = matrix_rows(engine, name);
    assert_eq!((rows.len(), rows[0].len()), (1, 1), "{name}");
    rows[0][0]
}

/// The unit cube with its centre and edge midpoints: the hull uses the 8
/// corners only, in 12 triangles, and encloses exactly 1 (SciPy: vertices
/// 1-8, 12 triangles, volume 1.0). convhull gives the same answer for a
/// matrix and for three vectors; the reversed set shows that the answer does
/// not depend on meeting the corners first.
#[test]
fn hull3_of_a_cube_uses_only_its_corners() {
    let mut e = Engine::default();
    e.execute(HULL_CUBE).unwrap();
    assert_eq!(matrix_rows(&e, "P").len(), 21);
    e.execute(
        "[K, V] = convhulln(P); [K2, V2] = convhull(P); \
         [K3, V3] = convhull(P(:,1), P(:,2), P(:,3)); K4 = convhulln(P); \
         same = double(isequal(K, K2, K3, K4) && V == V2 && V == V3);",
    )
    .unwrap();
    assert_eq!(scalar(&e, "same"), 1.);
    let s = hull_surface(&e, "P", "K");
    assert_eq!(s.vertices, (1..=8).collect::<Vec<_>>());
    assert_eq!((s.triangles, s.edges), (12, 18));
    assert_eq!(s.smallest_area, 0.5);
    assert_eq!(scalar(&e, "V"), 1.);
    assert_eq!(s.volume, 1.);

    e.execute("Q = P(end:-1:1, :); [K, V] = convhulln(Q);")
        .unwrap();
    let s = hull_surface(&e, "Q", "K");
    assert_eq!(s.vertices, (14..=21).collect::<Vec<_>>());
    assert_eq!((s.triangles, s.smallest_area), (12, 0.5));
    assert_eq!(scalar(&e, "V"), 1.);
}

/// The soccer ball: 60 vertices, 12 pentagons in 3 triangles and 20 hexagons
/// in 4, so 116 triangles on 90 + 2*12 + 3*20 = 174 edges, every one of the
/// ball's 90 edges among them (SciPy: 60 vertices, 116 triangles, volume
/// 3.633415364743332).
#[test]
fn hull3_of_bucky_splits_pentagons_and_hexagons() {
    let mut e = Engine::default();
    e.execute("[b, n] = bucky; [K, V] = convhulln(n); [i, j] = find(b); E = [i j];")
        .unwrap();
    let s = hull_surface(&e, "n", "K");
    assert_eq!(s.vertices, (1..=60).collect::<Vec<_>>());
    assert_eq!((s.triangles, s.edges), (116, 174));
    let volume = scalar(&e, "V");
    assert!(
        within(volume, 3.633415364743332, 1e-12),
        "volume {volume:?}"
    );
    assert!(
        within(s.volume, volume, 1e-12),
        "{} against {volume}",
        s.volume
    );
    // Any split of a regular hexagon cuts an ear off, the smallest triangle
    // there is: sqrt(3)/4 a^2 with the ball's edge a^2 = 16/(58 + 18 sqrt(5))
    // (0.0705; a pentagon's ear has 0.0774). Nothing is thinner than that.
    let ear = 3f64.sqrt() * 4. / (58. + 18. * 5f64.sqrt());
    assert!(
        within(s.smallest_area, ear, 1e-12),
        "smallest area {}",
        s.smallest_area
    );
    let sides: std::collections::BTreeSet<(usize, usize)> = matrix_rows(&e, "K")
        .iter()
        .flat_map(|t| {
            (0..3)
                .map(|i| (t[i] as usize, t[(i + 1) % 3] as usize))
                .collect::<Vec<_>>()
        })
        .collect();
    let ball = matrix_rows(&e, "E");
    assert_eq!(ball.len(), 180);
    for edge in ball {
        assert!(
            sides.contains(&(edge[0] as usize, edge[1] as usize)),
            "{edge:?}"
        );
    }
}

/// 200 scattered points: the vertex set and the volume are SciPy's (47
/// vertices, 90 triangles, volume 0.7997156857201125).
#[test]
fn hull3_of_scattered_points_matches_scipy() {
    const VERTICES: [usize; 47] = [
        2, 8, 11, 12, 13, 14, 22, 23, 40, 46, 62, 63, 64, 65, 76, 83, 87, 90, 104, 105, 110, 111,
        113, 121, 124, 129, 133, 134, 135, 138, 141, 142, 143, 145, 151, 155, 164, 165, 166, 170,
        180, 181, 185, 189, 191, 197, 198,
    ];
    let mut e = Engine::default();
    e.execute(HULL_SCATTERED).unwrap();
    let points = matrix_rows(&e, "P");
    // The reference script's first and last rows: the inputs are the same doubles.
    assert_eq!(
        points[0],
        [0.5710145559026928, 0.6472230188768464, 0.16124692846147667]
    );
    assert_eq!(
        points[199],
        [0.5435025419776806, 0.5158660824018838, 0.13135842752240526]
    );
    e.execute(
        "[K, V] = convhulln(P); [K2, V2] = convhull(P); same = double(isequal(K, K2) && V == V2);",
    )
    .unwrap();
    assert_eq!(scalar(&e, "same"), 1.);
    let s = hull_surface(&e, "P", "K");
    assert_eq!(s.vertices, VERTICES);
    assert_eq!((s.triangles, s.edges), (90, 135));
    let volume = scalar(&e, "V");
    assert!(
        within(volume, 0.7997156857201125, 1e-12),
        "volume {volume:?}"
    );
    assert!(
        within(s.volume, volume, 1e-12),
        "{} against {volume}",
        s.volume
    );
}

/// Duplicates and points on faces, edges and inside are not hull vertices,
/// exactly (a lattice; SciPy: vertices 1 4 13 16 49 52 61 64, 12 triangles,
/// volume 27) and within the tolerance (the cube set of the first test,
/// turned and moved; SciPy: vertices 1-8, 12 triangles, volume
/// 0.9999999999999996).
#[test]
fn hull3_ignores_duplicates_and_points_on_faces() {
    let mut e = Engine::default();
    e.execute(
        "[x, y, z] = ndgrid(0:3, 0:3, 0:3); P = [x(:) y(:) z(:)]; [K, V] = convhulln(P); \
         D = [P; P(end:-1:1, :); P]; [KD, VD] = convhulln(D);",
    )
    .unwrap();
    let s = hull_surface(&e, "P", "K");
    assert_eq!(s.vertices, [1, 4, 13, 16, 49, 52, 61, 64]);
    assert_eq!((s.triangles, s.smallest_area), (12, 4.5));
    assert_eq!(scalar(&e, "V"), 27.);
    // Three copies of every point: still 8 corners and 12 whole triangles.
    let s = hull_surface(&e, "D", "KD");
    assert_eq!(
        (s.vertices.len(), s.triangles, s.smallest_area),
        (8, 12, 4.5)
    );
    assert_eq!(scalar(&e, "VD"), 27.);

    e.execute(HULL_CUBE).unwrap();
    e.execute(
        "Rz = [cos(0.3) -sin(0.3) 0; sin(0.3) cos(0.3) 0; 0 0 1]; \
         Ry = [cos(0.7) 0 sin(0.7); 0 1 0; -sin(0.7) 0 cos(0.7)]; \
         Rx = [1 0 0; 0 cos(1.1) -sin(1.1); 0 sin(1.1) cos(1.1)]; \
         T = P * (Rz * Ry * Rx)' + repmat([10 20 30], 21, 1); [K, V] = convhulln(T);",
    )
    .unwrap();
    let s = hull_surface(&e, "T", "K");
    assert_eq!(s.vertices, (1..=8).collect::<Vec<_>>());
    assert_eq!(s.triangles, 12);
    assert!((s.smallest_area - 0.5).abs() < 1e-12, "{}", s.smallest_area);
    assert!(within(scalar(&e, "V"), 1., 1e-12), "{:?}", scalar(&e, "V"));
}

/// Not reference-checked (the expected corners are known by construction):
/// the answer does not depend on the size or the place of the set, points
/// within rounding of one another are one corner, and a face with many
/// corners is split without slivers.
#[test]
fn hull3_handles_scale_offsets_near_duplicates_and_polygon_faces() {
    let mut e = Engine::default();
    e.execute(HULL_CUBE).unwrap();
    // 0, 0.5 and 1 times these factors are exact; so are 1e6 + 0.5 and 1e6 + 1.
    for (transform, volume) in [
        ("P * 1e-100", 1e-300),
        ("P * 1e100", 1e300),
        ("P * 1e300", f64::INFINITY),
        ("P * 1e-320", 0.),
        ("P + 1e6", 1.),
        ("-3 * P - 1e9", 27.),
    ] {
        e.execute(&format!("T = {transform}; [K, V] = convhulln(T);"))
            .unwrap();
        let rows = matrix_rows(&e, "K");
        let mut used: Vec<usize> = rows.iter().flatten().map(|v| *v as usize).collect();
        used.sort_unstable();
        used.dedup();
        assert_eq!((rows.len(), used), (12, (1..=8).collect()), "{transform}");
        assert!(
            within(scalar(&e, "V"), volume, 1e-12),
            "{transform}: {:?}",
            scalar(&e, "V")
        );
    }
    // Every point twice, the copies a few rounding steps away.
    e.execute(
        "N = reshape(sin(1:63), 21, 3) * 1e-15; D = [P + N; P - N(end:-1:1, :)];          [K, V] = convhulln(D);",
    )
    .unwrap();
    let s = hull_surface(&e, "D", "K");
    assert_eq!((s.vertices.len(), s.triangles), (8, 12));
    assert!((s.smallest_area - 0.5).abs() < 1e-12, "{}", s.smallest_area);
    assert!(within(scalar(&e, "V"), 1., 1e-12));

    // A prism over a regular 32-gon with points on its faces: 64 corners,
    // 2 * 30 triangles on the ends and 2 * 32 on the sides.
    e.execute(
        "t = (0:31)' * 2 * pi / 32; c = [cos(t) sin(t)];          P = [c zeros(32, 1); c ones(32, 1); 0 0 0; 0 0 1; 0.3 0.2 0; 0.5 * c(1:5, :) ones(5, 1);          (c(1:8, :) + c(2:9, :)) / 2 zeros(8, 1)]; [K, V] = convhulln(P);",
    )
    .unwrap();
    let s = hull_surface(&e, "P", "K");
    assert_eq!(s.vertices, (1..=64).collect::<Vec<_>>());
    assert_eq!((s.triangles, s.edges), (124, 186));
    assert!(within(
        scalar(&e, "V"),
        16. * (std::f64::consts::PI / 16.).sin(),
        1e-12
    ));
    // The thinnest triangle any split has: an ear of the 32-gon.
    let ear = (std::f64::consts::PI / 16.).sin() * (1. - (std::f64::consts::PI / 16.).cos());
    assert!(
        within(s.smallest_area, ear, 1e-9),
        "{} against {ear}",
        s.smallest_area
    );

    // A corner that is almost straight (1e-12 is a distance, not rounding)
    // joins the far corners; cutting it off would leave a triangle of 2.5e-13.
    e.execute("P = [0 0 0; 1 0 0; 1 1 0; 0 1 0; 0.5 -1e-12 0; 0.5 0.5 1]; [K, V] = convhulln(P);")
        .unwrap();
    let s = hull_surface(&e, "P", "K");
    assert_eq!((s.vertices.len(), s.triangles), (6, 8));
    assert!(s.smallest_area > 0.24, "smallest area {}", s.smallest_area);
}

/// Fewer than four distinct points, collinear and coplanar sets (exactly and
/// within rounding) and non-finite coordinates are refused with their reason.
#[test]
fn hull3_rejects_degenerate_inputs() {
    let finite = "Data points must be finite.";
    for (source, want) in [
        ("convhulln([0 0 0; 1 0 0; 0 1 0])", HULL_TOO_FEW),
        ("convhull([0 0 0; 1 0 0; 0 1 0])", HULL_TOO_FEW),
        ("convhull([0 1 0], [0 0 1], [0 0 0])", HULL_TOO_FEW),
        ("convhulln(zeros(0, 3))", HULL_TOO_FEW),
        (
            "convhulln([0 0 0; 1 0 0; 0 1 0; 0 1 0; 0 0 0])",
            HULL_TOO_FEW,
        ),
        ("convhulln(ones(6, 3))", HULL_TOO_FEW),
        ("convhulln(zeros(6, 3))", HULL_TOO_FEW),
        (
            "convhulln([0 0 0; 1 1 1; 2 2 2; 3 3 3; 0.5 0.5 0.5])",
            HULL_FLAT,
        ),
        ("convhull([0 0 0; 1 2 3; 2 4 6; -1 -2 -3])", HULL_FLAT),
        (
            "convhulln([0 0 0; 1 0 0; 0 1 0; 1 1 0; 0.3 0.4 0])",
            HULL_FLAT,
        ),
        (
            "convhull([0 1 0 1 0.3], [0 0 1 1 0.4], [5 5 5 5 5])",
            HULL_FLAT,
        ),
        // On the plane x + y + z = 1, where thirds and tenths are rounded.
        (
            "convhulln([1 0 0; 0 1 0; 0 0 1; 1/3 1/3 1/3; 0.2 0.3 0.5; 0.7 0.2 0.1])",
            HULL_FLAT,
        ),
        ("convhulln([0 0 0; 1 0 0; 0 1 0; 0 0 NaN; 1 1 1])", finite),
        ("convhulln([0 0 0; 1 0 0; 0 1 0; 0 0 1; Inf 1 1])", finite),
        ("convhull([0 1 0 0 1], [0 0 1 0 1], [0 0 0 1 -Inf])", finite),
        ("convhull([0 0 0; 1 0 0; NaN 1 0])", finite),
        (
            "convhull([0 1 0 0], [0 0 1 0], [0 0 0])",
            "Data point coordinates have inconsistent dimension.",
        ),
        (
            "convhulln(ones(5, 4))",
            "convhulln supports points in two or three dimensions; X has 4 columns.",
        ),
        ("convhulln(1i * ones(5, 3))", "X must be real"),
        (
            "convhulln(zeros(200001, 3))",
            "X has more than 200000 points",
        ),
        (
            "z = zeros(1, 200001); convhull(z, z, z)",
            "convhull supports up to 200000 points",
        ),
    ] {
        let error = refused(source);
        assert_eq!(unplaced(&error), want, "{source}");
    }
    // A hull exists as soon as one point leaves the plane by more than rounding.
    let mut e = Engine::default();
    e.execute("P = [0 0 0; 1 0 0; 0 1 0; 1 1 0; 0.3 0.4 1e-9]; [K, V] = convhulln(P);")
        .unwrap();
    let s = hull_surface(&e, "P", "K");
    assert_eq!((s.vertices.len(), s.triangles), (5, 6));
    assert!(within(scalar(&e, "V"), 1e-9 / 3., 1e-12));
}

/// Not reference-checked: the surface rules alone decide. Small sets drawn
/// from a 4x4x4 lattice are full of duplicate, collinear and coplanar points;
/// with integer coordinates every test below is exact. A hull vertex must be
/// a corner (the normals of its triangles span space), and the same set
/// turned by an inexact rotation must keep the same number of corners.
#[test]
fn hull3_of_lattice_subsets_is_closed_convex_and_minimal() {
    let mut state = 12345u64;
    let mut draw = |n: u64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) % n
    };
    let mut e = Engine::default();
    let (mut hulls, mut flat) = (0, 0);
    for case in 0..400 {
        let count = 4 + draw(28);
        let rows: Vec<String> = (0..count)
            .map(|_| format!("{} {} {}", draw(4), draw(4), draw(4)))
            .collect();
        let source = format!("P = [{}];", rows.join("; "));
        e.execute(&source).unwrap();
        match e.execute("[K, V] = convhulln(P);") {
            Err(error) => {
                let why = unplaced(&error);
                assert!(
                    why == HULL_FLAT || why == HULL_TOO_FEW,
                    "case {case} {source}: {error}"
                );
                // Really flat: every point is in the plane of some three, or all are collinear.
                e.execute("r = rank(P(2:end, :) - repmat(P(1, :), size(P, 1) - 1, 1));")
                    .unwrap();
                assert!(scalar(&e, "r") < 3., "case {case} {source}: {error}");
                flat += 1;
            }
            Ok(_) => {
                hulls += 1;
                // Shown only when the test fails.
                println!("case {case}: {source}");
                let s = hull_surface(&e, "P", "K");
                assert!(s.smallest_area >= 0.5, "case {case} {source}");
                assert!(
                    within(s.volume, scalar(&e, "V"), 1e-12),
                    "case {case} {source}"
                );
                let points = matrix_rows(&e, "P");
                let triangles = matrix_rows(&e, "K");
                for v in &s.vertices {
                    let normals: Vec<[i64; 3]> = triangles
                        .iter()
                        .filter(|t| t.contains(&(*v as f64)))
                        .map(|t| {
                            let at = |i: usize| -> [i64; 3] {
                                std::array::from_fn(|k| points[t[i] as usize - 1][k] as i64)
                            };
                            let (a, b, c) = (at(0), at(1), at(2));
                            let (u, w): ([i64; 3], [i64; 3]) = (
                                std::array::from_fn(|k| b[k] - a[k]),
                                std::array::from_fn(|k| c[k] - a[k]),
                            );
                            [
                                u[1] * w[2] - u[2] * w[1],
                                u[2] * w[0] - u[0] * w[2],
                                u[0] * w[1] - u[1] * w[0],
                            ]
                        })
                        .collect();
                    let spans = normals.iter().any(|a| {
                        normals.iter().any(|b| {
                            normals.iter().any(|c| {
                                a[0] * (b[1] * c[2] - b[2] * c[1])
                                    - a[1] * (b[0] * c[2] - b[2] * c[0])
                                    + a[2] * (b[0] * c[1] - b[1] * c[0])
                                    != 0
                            })
                        })
                    });
                    assert!(spans, "case {case} {source}: vertex {v} is not a corner");
                }
                // The same set, turned and moved: rounding puts points beside
                // their faces and edges, the corners stay the same.
                e.execute(
                    "R = [cos(0.3) -sin(0.3) 0; sin(0.3) cos(0.3) 0; 0 0 1] * \
                     [cos(0.7) 0 sin(0.7); 0 1 0; -sin(0.7) 0 cos(0.7)] * \
                     [1 0 0; 0 cos(1.1) -sin(1.1); 0 sin(1.1) cos(1.1)]; \
                     T = P * R' + repmat([10 20 30], size(P, 1), 1); [KT, VT] = convhulln(T);",
                )
                .unwrap();
                let turned = hull_surface(&e, "T", "KT");
                assert_eq!(
                    turned.vertices.len(),
                    s.vertices.len(),
                    "case {case} {source}: corners of the turned set"
                );
                assert_eq!(turned.triangles, s.triangles, "case {case} {source}");
                assert!(turned.smallest_area > 0.49, "case {case} {source}");
                assert!(
                    within(scalar(&e, "VT"), s.volume, 1e-11),
                    "case {case} {source}"
                );
            }
        }
    }
    assert!(hulls > 300 && flat > 0, "{hulls} hulls, {flat} flat sets");
}

/// 100000 points in a cube: better than quadratic (the task's limit is 2 s
/// in the gate profile; measured 2026-11-21 in docs/tasks notes), and points
/// on a sphere, where every point is a vertex.
#[test]
fn hull3_of_many_points_is_fast() {
    let mut e = Engine::default();
    e.execute("P = rand(100000, 3);").unwrap();
    let started = std::time::Instant::now();
    e.execute("[K, V] = convhulln(P);").unwrap();
    let elapsed = started.elapsed();
    let s = hull_surface(&e, "P", "K");
    assert_eq!(s.triangles, 2 * s.vertices.len() - 4);
    assert!(scalar(&e, "V") > 0.95 && scalar(&e, "V") < 1.);
    assert!(within(s.volume, scalar(&e, "V"), 1e-12));
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "convhulln of 100000 points took {elapsed:?}"
    );

    e.execute(
        "Q = randn(20000, 3); Q = Q ./ repmat(sqrt(sum(Q.^2, 2)), 1, 3); [K, V] = convhulln(Q);",
    )
    .unwrap();
    e.execute("x = [size(K, 1), numel(unique(K)), V];").unwrap();
    let x = values(&mut e, "x = x;");
    assert_eq!((x[0], x[1]), (2. * 20000. - 4., 20000.));
    assert!(
        x[2] > 4.18 && x[2] < 4. * std::f64::consts::PI / 3.,
        "volume {}",
        x[2]
    );
}
