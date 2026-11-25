//! contourc, contour, contourf and clabel.
//! Reference: GNU Octave 10.3's contourc. tool/refs/contour/make_refs.m wrote
//! tests/fixtures/contour/*.json (inputs, Octave's level vector and C matrix);
//! levels and counts must match exactly, points within 1e-12.
use matrust::{Array, Engine, Matrix, plot::PlotKind};
use std::collections::BTreeMap;

fn run(source: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(source)
        .unwrap_or_else(|err| panic!("{source}: {err}"));
    e
}
fn error_of(source: &str) -> String {
    match Engine::default().execute(source) {
        Ok(_) => panic!("{source} should fail"),
        Err(e) => e.to_string(),
    }
}
fn matrix(e: &Engine, name: &str) -> Matrix {
    match e.array(name) {
        Some(Array::Numeric(m)) => m,
        other => panic!("{name} is not numeric: {other:?}"),
    }
}
fn num(e: &Engine, name: &str) -> Vec<f64> {
    matrix(e, name).data().to_vec()
}
fn text(e: &Engine, name: &str) -> String {
    match e.array(name) {
        Some(Array::Character(c)) => c.to_utf8(&e.cancellation()).unwrap(),
        other => panic!("{name} is not text: {other:?}"),
    }
}

// ------------------------------------------------------------- fixtures

struct Case {
    name: &'static str,
    x: Option<Matrix>,
    y: Option<Matrix>,
    z: Matrix,
    arg: Vec<f64>,
    lev: Vec<f64>,
    c: [Vec<f64>; 2],
}

fn values(v: &serde_json::Value) -> Vec<f64> {
    match v {
        serde_json::Value::Array(items) => items
            .iter()
            .map(|x| x.as_f64().unwrap_or(f64::NAN))
            .collect(),
        serde_json::Value::Null => vec![f64::NAN],
        x => vec![x.as_f64().unwrap()],
    }
}
/// A matrix written as an array of rows (empty: None).
fn rows(v: &serde_json::Value) -> Option<Matrix> {
    let rows: Vec<Vec<f64>> = v.as_array()?.iter().map(values).collect();
    if rows.is_empty() {
        return None;
    }
    let (r, c) = (rows.len(), rows[0].len());
    let data = (0..c)
        .flat_map(|j| rows.iter().map(move |row| row[j]))
        .collect();
    Some(Matrix::from_column_vec(r, c, data).unwrap())
}

fn fixture(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/tests/fixtures/contour/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap()
}

fn case(name: &'static str) -> Case {
    let json = fixture(name);
    Case {
        name,
        x: rows(&json["x"]),
        y: rows(&json["y"]),
        z: rows(&json["z"]).unwrap(),
        arg: values(&json["arg"]),
        lev: values(&json["lev"]),
        c: [values(&json["c1"]), values(&json["c2"])],
    }
}

const CASES: &[&str] = &[
    "peaks20_default",
    "peaks20_n1",
    "peaks20_n3",
    "peaks20_n7",
    "peaks20_v",
    "peaks20_kk",
    "peaks20_outside",
    "saddle5",
    "saddle5_v",
    "checker",
    "checker_ramp",
    "nan_hole",
    "nonuniform",
    "meshgrid",
    "curvilinear",
    "constant",
];

/// An engine holding the case's inputs as Z, x, y and a.
fn load(c: &Case) -> (Engine, String) {
    let mut e = Engine::default();
    let mut vars = BTreeMap::new();
    vars.insert("Z".to_owned(), Array::Numeric(c.z.clone()));
    let mut call = String::from("contourc(");
    if let (Some(x), Some(y)) = (&c.x, &c.y) {
        vars.insert("x".to_owned(), Array::Numeric(x.clone()));
        vars.insert("y".to_owned(), Array::Numeric(y.clone()));
        call.push_str("x, y, ");
    }
    call.push('Z');
    if !c.arg.is_empty() {
        let a = Matrix::from_column_vec(1, c.arg.len(), c.arg.clone()).unwrap();
        vars.insert("a".to_owned(), Array::Numeric(a));
        call.push_str(", a");
    }
    call.push(')');
    e.merge_workspace(vars).unwrap();
    (e, call)
}

fn assert_c_matches(name: &str, got: &Matrix, want: &[Vec<f64>; 2], lev: &[f64]) {
    assert_eq!(got.rows(), 2, "{name}");
    assert_eq!(got.cols(), want[0].len(), "{name}: number of columns");
    let mut k = 0;
    while k < got.cols() {
        let (level, n) = (got.get(0, k), got.get(1, k));
        assert_eq!(
            level.to_bits(),
            want[0][k].to_bits(),
            "{name}: level at column {k}"
        );
        assert_eq!(n, want[1][k], "{name}: count at column {k}");
        assert!(
            lev.iter().any(|l| l.to_bits() == level.to_bits()),
            "{name}: {level} is not one of Octave's levels"
        );
        for j in k + 1..=k + n as usize {
            for (r, w) in [(0, &want[0]), (1, &want[1])] {
                let (a, b) = (got.get(r, j), w[j]);
                assert!(
                    (a - b).abs() <= 1e-12 * b.abs().max(1.),
                    "{name}: point column {j} row {r}: {a} vs {b}"
                );
            }
        }
        k += n as usize + 1;
    }
}

#[test]
fn contourc_matrices_equal_octave_for_every_fixture() {
    for name in CASES {
        let c = case(name);
        let (mut e, call) = load(&c);
        e.execute(&format!("C = {call};"))
            .unwrap_or_else(|err| panic!("{}: {call}: {err}", c.name));
        assert_c_matches(c.name, &matrix(&e, "C"), &c.c, &c.lev);
    }
}

#[test]
fn level_rule_is_octaves_linspace_without_its_ends() {
    // Default: 10 levels from linspace(min, max, 12); n: linspace(min, max, n+2).
    for (name, n) in [
        ("peaks20_default", 10),
        ("peaks20_n1", 1),
        ("peaks20_n3", 3),
        ("peaks20_n7", 7),
        ("saddle5", 10),
    ] {
        let c = case(name);
        assert_eq!(c.lev.len(), n, "{name}");
        let (mut e, _) = load(&c);
        let arg = if c.arg.is_empty() {
            String::new()
        } else {
            ", a".into()
        };
        e.execute(&format!("[~, h] = contour(Z{arg}); L = h.LevelList;"))
            .unwrap();
        let got = num(&e, "L");
        assert_eq!(got.len(), n, "{name}");
        for (a, b) in got.iter().zip(&c.lev) {
            assert_eq!(a.to_bits(), b.to_bits(), "{name}: {got:?} vs {:?}", c.lev);
        }
    }
    // [k k] is the single level k; an explicit v is sorted.
    let e = run(
        "[~, h] = contour(peaks(20), [1 1]); a = h.LevelList; [~, g] = contour(peaks(20), [3 -2 0]); b = g.LevelList;",
    );
    assert_eq!(num(&e, "a"), [1.]);
    assert_eq!(num(&e, "b"), [-2., 0., 3.]);
}

#[test]
fn constant_data_and_out_of_range_levels_give_no_lines() {
    let e = run(
        "C = contourc(3*ones(4,5)); s = size(C); [C2, h] = contour(3*ones(4,5)); t = h.Type; D = contourc(peaks(20), [20 30]); d = size(D);",
    );
    assert_eq!(num(&e, "s"), [2., 0.]);
    assert_eq!(num(&e, "d"), [2., 0.]);
    assert_eq!(text(&e, "t"), "contour");
    assert_eq!(matrix(&e, "C2").cols(), 0);
}

// ------------------------------------------------------------- drawing

/// The figure's series of a kind.
fn series_of(e: &Engine, kind: PlotKind) -> Vec<matrust::plot::Series> {
    e.figure
        .as_ref()
        .unwrap()
        .axes
        .series
        .iter()
        .filter(|s| s.kind == kind)
        .cloned()
        .collect()
}
/// The colormap row MATLAB's scaled colour mapping picks for v.
fn mapped(map: &Matrix, clim: [f64; 2], v: f64) -> [u8; 3] {
    let m = map.rows();
    let t = (v - clim[0]) / (clim[1] - clim[0]);
    let i = ((t * m as f64).floor().max(0.) as usize).min(m - 1);
    [0, 1, 2].map(|k| (map.get(i, k) * 255.).round() as u8)
}
/// Distinct levels in C that have at least one line, in order.
fn levels_with_lines(c: &Matrix) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    let mut k = 0;
    while k < c.cols() {
        if out.last() != Some(&c.get(0, k)) {
            out.push(c.get(0, k));
        }
        k += c.get(1, k) as usize + 1;
    }
    out
}
fn blocks(c: &Matrix) -> Vec<f64> {
    let mut out = Vec::new();
    let mut k = 0;
    while k < c.cols() {
        out.push(c.get(0, k));
        k += c.get(1, k) as usize + 1;
    }
    out
}

#[test]
fn contour_draws_one_coloured_line_per_level_and_colorbar_shows_the_levels() {
    let e = run(
        "Z = peaks(20); [C, h] = contour(Z); C0 = contourc(Z); L = h.LevelList; cm = colormap; \
         cb = colorbar; lim = cb.Limits; cl = clim; f = h.Fill; w = h.LineWidth; lc = h.LineColor;",
    );
    let c = matrix(&e, "C");
    assert_eq!(c.data(), matrix(&e, "C0").data());
    let lines = series_of(&e, PlotKind::Line);
    let levels = levels_with_lines(&c);
    assert_eq!(lines.len(), levels.len());
    let all = num(&e, "L");
    let clim = [all[0], *all.last().unwrap()];
    assert_eq!(num(&e, "lim"), clim);
    assert_eq!(num(&e, "cl"), clim);
    let map = matrix(&e, "cm");
    for (s, level) in lines.iter().zip(&levels) {
        assert_eq!(s.color, mapped(&map, clim, *level), "level {level}");
        assert_eq!(s.line_width, Some(0.5));
    }
    // Lowest and highest levels take the first and last colormap rows.
    assert_ne!(lines[0].color, lines.last().unwrap().color);
    assert_eq!(text(&e, "f"), "off");
    assert_eq!(num(&e, "w"), [0.5]);
    assert_eq!(text(&e, "lc"), "flat");
    // One legend entry: the helper lines carry empty names.
    let named = e
        .figure
        .as_ref()
        .unwrap()
        .axes
        .series
        .iter()
        .filter(|s| !s.label.is_empty())
        .count();
    assert_eq!(named, 1);
}

#[test]
fn line_spec_line_color_and_the_axes_colormap() {
    let e = run("[~, h] = contour(peaks(20), 4, 'r--');");
    for s in series_of(&e, PlotKind::Line) {
        assert_eq!(s.color, [255, 0, 0]);
        assert_eq!(s.pattern, matrust::plot::LinePattern::Dashed);
    }
    let e = run("[~, h] = contour(peaks(20), 'LineColor', [0 0.5 1], 'LineWidth', 2);");
    for s in series_of(&e, PlotKind::Line) {
        assert_eq!(s.color, [0, 128, 255]);
        assert_eq!(s.line_width, Some(2.));
    }
    let e = run("colormap(gray); contour(peaks(20));");
    for s in series_of(&e, PlotKind::Line) {
        assert!(
            s.color[0] == s.color[1] && s.color[1] == s.color[2],
            "{:?}",
            s.color
        );
    }
}

#[test]
fn set_level_list_redraws_through_get_set_and_dot_syntax() {
    let e = run(
        "[C, h] = contour(peaks(20)); set(h, 'LevelList', [0 1]); L = get(h, 'LevelList'); cl = clim; h.LineWidth = 1.5; w = h.LineWidth; \
         Zd = h.ZData; t = h.Type;",
    );
    assert_eq!(num(&e, "L"), [0., 1.]);
    assert_eq!(num(&e, "cl"), [0., 1.]);
    let lines = series_of(&e, PlotKind::Line);
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|s| s.line_width == Some(1.5)));
    assert_eq!(matrix(&e, "Zd").rows(), 20);
    assert_eq!(text(&e, "t"), "contour");
    assert!(error_of("[~, h] = contour(peaks(5)); set(h, 'Fill', 'on');").contains("read-only"));
}

#[test]
fn show_text_and_clabel_add_one_label_per_line() {
    let e = run("[C, h] = contour(peaks(20), 'ShowText', 'on'); s = h.ShowText;");
    let c = matrix(&e, "C");
    let texts = &e.figure.as_ref().unwrap().axes.extras.texts;
    let want: Vec<String> = blocks(&c).iter().map(|l| g(*l)).collect();
    assert_eq!(
        texts.iter().map(|t| t.string.clone()).collect::<Vec<_>>(),
        want
    );
    assert_eq!(text(&e, "s"), "on");
    assert!(texts.iter().all(|t| t.font_size == 10.));

    let e = run(
        "[C, h] = contour(peaks(20), [-2 0 2]); t = clabel(C, h); n = numel(t); s = get(t(1), 'String'); u = clabel(C, h, 0); m = numel(u);",
    );
    let c = matrix(&e, "C");
    let b = blocks(&c);
    assert_eq!(num(&e, "n"), [b.len() as f64]);
    assert_eq!(text(&e, "s"), g(b[0]));
    assert_eq!(
        num(&e, "m"),
        [b.iter().filter(|&&l| l == 0.).count() as f64]
    );
    // Redrawing keeps only the current labels.
    assert_eq!(
        e.figure.as_ref().unwrap().axes.extras.texts.len(),
        b.iter().filter(|&&l| l == 0.).count()
    );
    // A label sits on its line: the midpoint of a closed or open polyline.
    let e = run(
        "C = contourc([1 2; 3 4], [2.5 2.5]); t = clabel(C); p = get(t, 'Position'); s = get(t, 'String');",
    );
    let p = num(&e, "p");
    assert!(
        (p[0] - 1.5).abs() < 1e-12 && (p[1] - 1.5).abs() < 1e-12,
        "{p:?}"
    );
    assert_eq!(text(&e, "s"), "2.5");
}

fn g(v: f64) -> String {
    // %g for the levels used here (6 significant digits, trailing zeros cut).
    let s = format!(
        "{v:.*}",
        (5 - v.abs().log10().floor() as i32).max(0) as usize
    );
    if v == 0. {
        return "0".into();
    }
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').into()
    } else {
        s
    }
}

/// Area of rectangle (4-corner) and triangle (3-corner) patch faces.
fn face_area(s: &matrust::plot::Series) -> f64 {
    let n = s.patch.unwrap().vertices_per_face();
    let p: Vec<[f64; 2]> = s.points.iter().collect();
    p.chunks(n)
        .map(|f| {
            if n == 4 {
                ((f[2][0] - f[0][0]) * (f[2][1] - f[0][1])).abs()
            } else {
                0.5 * ((f[1][0] - f[0][0]) * (f[2][1] - f[0][1])
                    - (f[1][1] - f[0][1]) * (f[2][0] - f[0][0]))
                    .abs()
            }
        })
        .sum()
}

/// Octave's contourf answer for one grid (tool/refs/contour/make_refs.m):
/// LevelList, CLim and the 1-based colormap row of every filled band for a
/// 256-row colormap (graphics.cc convert_cdata).
struct Filled {
    z: Matrix,
    arg: Vec<f64>,
    lev: Vec<f64>,
    clim: Vec<f64>,
    rows256: Vec<f64>,
}
fn filled_case(name: &str) -> Filled {
    let json = fixture(name);
    Filled {
        z: rows(&json["z"]).unwrap(),
        arg: values(&json["arg"]),
        lev: values(&json["lev"]),
        clim: values(&json["clim"]),
        rows256: values(&json["rows256"]),
    }
}
/// contourf(Z, a, extra) on the case's Z and argument.
fn filled_run(c: &Filled, extra: &str) -> Engine {
    let mut e = Engine::default();
    let a = Matrix::from_column_vec(1, c.arg.len(), c.arg.clone()).unwrap();
    e.merge_workspace(BTreeMap::from([
        ("Z".to_owned(), Array::Numeric(c.z.clone())),
        ("a".to_owned(), Array::Numeric(a)),
    ]))
    .unwrap();
    e.execute(&format!(
        "[C, h] = contourf(Z, a{extra}); L = h.LevelList; cl = clim; cm = colormap; f = h.Fill; lc = h.LineColor;"
    ))
    .unwrap();
    e
}
fn band_colours(e: &Engine) -> std::collections::BTreeSet<[u8; 3]> {
    series_of(e, PlotKind::Patch)
        .iter()
        .map(|p| p.color)
        .collect()
}

#[test]
fn contourf_levels_clim_and_band_colours_equal_octave() {
    // A count n: min(Z) leads the n contourc levels, CLim is [min(Z) max
    // level] and every band, the lowest included, has its own colormap row.
    let c = filled_case("contourf_peaks20_n5");
    let e = filled_run(&c, "");
    assert_eq!(num(&e, "L"), c.lev);
    assert_eq!(num(&e, "cl"), c.clim);
    assert_eq!(text(&e, "f"), "on");
    assert_eq!(num(&e, "lc"), [0., 0., 0.]);
    let map = matrix(&e, "cm");
    assert_eq!(map.rows(), 256);
    let row = |r: f64| [0, 1, 2].map(|k| (map.get(r as usize - 1, k) * 255.).round() as u8);
    let want: std::collections::BTreeSet<[u8; 3]> = c.rows256.iter().map(|&r| row(r)).collect();
    assert_eq!(want.len(), 6, "Octave: 6 bands, 6 rows");
    assert_eq!(band_colours(&e), want);
    // The bands tile the grid: their areas sum to the 19-by-19 cells.
    let patches = series_of(&e, PlotKind::Patch);
    let area: f64 = patches.iter().map(face_area).sum();
    assert!((area - 361.).abs() < 1e-9, "{area}");
    // min(Z) draws no line: C is contourc's.
    let mut e = e;
    e.execute("C0 = contourc(Z, a);").unwrap();
    let lines = levels_with_lines(&matrix(&e, "C"));
    assert!(!lines.contains(&c.lev[0]), "{lines:?}");
    assert_eq!(lines, c.lev[1..]);
    assert_eq!(matrix(&e, "C").data(), matrix(&e, "C0").data());
    // Lines on top, in black.
    let all = &e.figure.as_ref().unwrap().axes.series;
    let first_line = all.iter().position(|s| s.kind == PlotKind::Line).unwrap();
    assert!(all[first_line..].iter().all(|s| s.kind == PlotKind::Line));
    assert!(
        series_of(&e, PlotKind::Line)
            .iter()
            .all(|s| s.color == [0, 0, 0])
    );
    // 'LineColor', 'none': no lines, the lowest band still has its colour.
    let e2 = filled_run(&c, ", 'LineColor', 'none'");
    assert!(series_of(&e2, PlotKind::Line).is_empty());
    assert_eq!(band_colours(&e2), want);

    // Explicit levels: no min(Z), CLim spans them and the region below the
    // first level stays unfilled (Octave paints no patch there).
    let c = filled_case("contourf_peaks20_v");
    let e = filled_run(&c, "");
    assert_eq!(num(&e, "L"), c.lev);
    assert_eq!(num(&e, "cl"), c.clim);
    let want: std::collections::BTreeSet<[u8; 3]> = c.rows256.iter().map(|&r| row(r)).collect();
    assert_eq!(want.len(), 3);
    assert_eq!(band_colours(&e), want);
    // The unfilled part is exactly the lowest band of contourf(Z, [min(Z) v]).
    let area: f64 = series_of(&e, PlotKind::Patch).iter().map(face_area).sum();
    let zmin = c.z.data().iter().copied().fold(f64::INFINITY, f64::min);
    let full = filled_run(
        &Filled {
            arg: [vec![zmin], c.arg.clone()].concat(),
            ..filled_case("contourf_peaks20_v")
        },
        "",
    );
    let lowest = row(1.);
    let below: f64 = series_of(&full, PlotKind::Patch)
        .iter()
        .filter(|p| p.color == lowest)
        .map(face_area)
        .sum();
    assert!(below > 1., "{below}");
    assert!((area + below - 361.).abs() < 1e-9, "{area} + {below}");

    // More of Octave's answers (tool/refs/contour/probe_levels.m): LevelList, CLim.
    for (src, lev, cl) in [
        ("contourf(magic(4), 2)", vec![1., 6., 11.], [1., 11.]),
        ("contourf(magic(4), [7 7])", vec![7.], [6., 8.]),
        ("contourf(2 * ones(3))", vec![2.], [1., 3.]),
        // Octave's LevelList is [1 6 11] here; min(Z) has no line and matrust leaves it out.
        ("contour(magic(4), 2)", vec![6., 11.], [6., 11.]),
    ] {
        let e = run(&format!("[~, h] = {src}; L = h.LevelList; cl = clim;"));
        assert_eq!(num(&e, "L"), lev, "{src}");
        assert_eq!(num(&e, "cl"), cl, "{src}");
    }
    assert_eq!(band_colours(&run("contourf(2 * ones(3));")).len(), 1);
    // NaN ignored by min/max; a new LevelList on contourf is v's rule (no min(Z)).
    let e =
        run("z = magic(4); z(2, 2) = NaN; [~, h] = contourf(z, 1); L = h.LevelList; cl = clim;");
    assert_eq!(num(&e, "L"), [1., 8.5]);
    assert_eq!(num(&e, "cl"), [1., 8.5]);
    let e =
        run("[~, h] = contourf(magic(4)); set(h, 'LevelList', [0 8]); L = h.LevelList; cl = clim;");
    assert_eq!(num(&e, "L"), [0., 8.]);
    assert_eq!(num(&e, "cl"), [0., 8.]);
}

#[test]
fn contourf_leaves_nan_cells_empty_and_follows_meshes() {
    let c = case("nan_hole");
    let (mut e, _) = load(&c);
    e.execute("contourf(Z);").unwrap();
    let area: f64 = series_of(&e, PlotKind::Patch).iter().map(face_area).sum();
    // 11x11 cells minus the 4x4 cells that touch the 3x3 NaN block.
    assert!((area - (121. - 16.)).abs() < 1e-9, "{area}");
    let e =
        run("[X, Y] = meshgrid(linspace(0, 2, 9), linspace(-1, 1, 7)); contourf(X, Y, X.^2 - Y);");
    let area: f64 = series_of(&e, PlotKind::Patch).iter().map(face_area).sum();
    assert!((area - 4.).abs() < 1e-9, "{area}");
}

#[test]
fn contour_targets_an_axes_and_returns_handles() {
    let e = run(
        "subplot(1, 2, 1); plot(1:3); ax = gca; subplot(1, 2, 2); [C, h] = contour(ax, peaks(10), 3); t = h.Type; n = numel(C);",
    );
    assert_eq!(text(&e, "t"), "contour");
    assert!(num(&e, "n")[0] > 0.);
    // The contour replaced the plot in the first axes, not the current one.
    let f = e.figure.as_ref().unwrap();
    let lines: usize = f.all_axes().map(|(_, a)| a.series.len()).sum();
    assert_eq!(lines, 3, "three levels, nothing else");
}

#[test]
fn input_errors_use_matlab_style_messages() {
    for (source, message) in [
        ("contour(1:5)", "contour: Z must be at least a 2x2 matrix"),
        (
            "contourc([1 2 3])",
            "contourc: Z must be at least a 2x2 matrix",
        ),
        ("contour([1 2; 3 4] * 1i)", "contour: Z must be real"),
        (
            "contourf(1:3, 1:3, magic(4))",
            "contourf: the lengths of X and Y must match",
        ),
        (
            "contour(ones(3), ones(3), magic(4))",
            "contour: X and Y must be vectors",
        ),
        (
            "contour(zeros(1001, 1000))",
            "contour: Z exceeds 1000000 elements",
        ),
        (
            "contour(magic(4), 0)",
            "contour: the number of levels must be a positive integer",
        ),
        (
            "contour([1 Inf; 2 3])",
            "contour: Z must contain only finite values or NaN",
        ),
        ("contour('abc')", "contour: Z must be a real numeric matrix"),
        (
            "contourc(magic(4), 2.5)",
            "contourc: the number of levels must be a positive integer",
        ),
        (
            "contourf(peaks(1000), 20)",
            "contourf: the filled bands need more than",
        ),
        (
            "clabel([1 5; 2 3])",
            "clabel: C is not a valid contour matrix",
        ),
        ("clabel('abc')", "clabel: C must be a contour matrix"),
    ] {
        let error = error_of(source);
        assert!(error.contains(message), "{source}: {error}");
    }
    // A failed call leaves the figure as it was.
    let mut e = run("plot(1:3);");
    assert!(e.execute("contourf(peaks(1000), 20);").is_err());
    assert_eq!(e.figure.as_ref().unwrap().axes.series.len(), 1);
}

#[test]
fn svg_export_of_contourf_holds_band_polygons_and_line_paths() {
    let e = run("contourf(peaks(20), 4);");
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.matches("<polygon").count() > 0, "no band polygons");
    assert!(svg.matches("<path").count() > 0, "no line paths");
    let e = run("contour(peaks(20), 'ShowText', 'on');");
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.contains("data-matrust-text"));
}

#[test]
fn mrfig_keeps_the_drawn_lines_and_bands() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("contourf(peaks(20), 4); savefig('c.mrfig');")
        .unwrap();
    let before = e.figure.clone().unwrap();
    let decoded = matrust::files::load_figure(&dir.path().join("c.mrfig")).unwrap();
    assert_eq!(before.axes.series.len(), decoded.axes.series.len());
    for (a, b) in before.axes.series.iter().zip(&decoded.axes.series) {
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.color, b.color);
        let bits = |s: &matrust::plot::Series| -> Vec<[u64; 2]> {
            s.points.iter().map(|p| p.map(f64::to_bits)).collect()
        };
        assert_eq!(bits(a), bits(b));
    }
    assert_eq!(before.svg(), decoded.svg());
}

// ------------------------------------------------- fix round 1 regressions

/// Line series that draw something (the placeholder of an empty contour is NaN).
fn drawn_lines(e: &Engine) -> Vec<matrust::plot::Series> {
    series_of(e, PlotKind::Line)
        .into_iter()
        .filter(|s| s.points.iter().any(|p| !p[0].is_nan()))
        .collect()
}

#[test]
fn clabel_rejects_a_malformed_contour_matrix_without_panicking() {
    for source in [
        "clabel([1; 0])",             // a block without points
        "clabel([1 2 3; 5 4 4])",     // a count beyond the matrix
        "clabel([1 2 3; 1.5 4 4])",   // a count that is not whole
        "clabel([1 2; NaN 3])",       // no count at all
        "clabel([1 2 3; -1 4 4])",    // a negative count
        "clabel([1 2 3; Inf 4 4])",   // an infinite count
        "clabel([1 2 3 4 5])",        // one row
        "clabel(ones(3, 4))",         // three rows
        "clabel([2 1 1 5; 2 1 1 0])", // a valid block, then one without points
        "[C, h] = contour(peaks(10)); clabel([1; 0], h)",
        "[C, h] = contour(peaks(10)); clabel([1 2 3; 5 4 4], h, 1)",
    ] {
        let error = error_of(source);
        assert!(
            error.contains("clabel: C is not a valid contour matrix"),
            "{source}: {error}"
        );
    }
    // Well-formed ones still label: one block of two points, an empty C.
    let e = run("t = clabel([1 2 3; 2 4 5]); n = numel(t); u = clabel(zeros(2, 0)); m = numel(u);");
    assert_eq!(num(&e, "n"), [1.]);
    assert_eq!(num(&e, "m"), [0.]);
    // A failed clabel leaves the figure as it was.
    let mut e = run("plot(1:3);");
    assert!(e.execute("clabel([1; 0]);").is_err());
    assert!(e.figure.as_ref().unwrap().axes.extras.texts.is_empty());
}

#[test]
fn clabel_with_a_handle_labels_the_lines_of_the_given_c() {
    // The contour's own C labels like the contour; another C labels its own lines.
    let e = run(
        "[C, h] = contour(peaks(20), [-2 0 2]); t = clabel(C, h); n = numel(t); \
         D = contourc([1 2; 3 4], [2.5 2.5]); u = clabel(D, h); m = numel(u); p = get(u, 'Position');",
    );
    let c = matrix(&e, "C");
    assert_eq!(num(&e, "n"), [blocks(&c).len() as f64]);
    assert_eq!(num(&e, "m"), [1.]);
    let p = num(&e, "p");
    assert!(
        (p[0] - 1.5).abs() < 1e-12 && (p[1] - 1.5).abs() < 1e-12,
        "{p:?}"
    );
    assert_eq!(e.figure.as_ref().unwrap().axes.extras.texts.len(), 1);
}

#[test]
fn line_style_none_hides_the_lines_and_leaves_line_color_alone() {
    let e = run(
        "[C, h] = contour(peaks(20), 4); set(h, 'LineStyle', 'none'); a = h.LineStyle; c1 = h.LineColor;",
    );
    assert!(drawn_lines(&e).is_empty());
    assert_eq!(text(&e, "a"), "none");
    assert_eq!(text(&e, "c1"), "flat");
    // A later '-' brings the lines back, coloured by their levels.
    let mut e = run(
        "[C, h] = contour(peaks(20), 4); set(h, 'LineStyle', 'none'); set(h, 'LineStyle', '-'); a = h.LineStyle; c1 = h.LineColor;",
    );
    let levels = levels_with_lines(&matrix(&e, "C"));
    assert_eq!(drawn_lines(&e).len(), levels.len());
    assert_eq!(text(&e, "a"), "-");
    assert_eq!(text(&e, "c1"), "flat");
    assert_ne!(
        drawn_lines(&e)[0].color,
        drawn_lines(&e).last().unwrap().color
    );
    e.execute("set(h, 'LineStyle', '--');").unwrap();
    assert!(
        drawn_lines(&e)
            .iter()
            .all(|s| s.pattern == matrust::plot::LinePattern::Dashed)
    );
    // An explicit colour survives the round trip.
    let e = run(
        "[~, h] = contour(peaks(20), 4, 'LineColor', [1 0 0]); set(h, 'LineStyle', 'none'); set(h, 'LineStyle', ':'); c = h.LineColor;",
    );
    assert_eq!(num(&e, "c"), [1., 0., 0.]);
    assert!(drawn_lines(&e).iter().all(|s| s.color == [255, 0, 0]));
    // LineColor 'none' is its own switch: LineStyle '-' does not undo it.
    let e = run("[~, h] = contourf(peaks(20), 4); set(h, 'LineStyle', '-'); c = h.LineColor;");
    assert_eq!(num(&e, "c"), [0., 0., 0.]);
    let e = run(
        "[~, h] = contourf(peaks(20), 4, 'LineColor', 'none'); set(h, 'LineStyle', '-'); c = h.LineColor;",
    );
    assert!(drawn_lines(&e).is_empty());
    assert_eq!(text(&e, "c"), "none");
    // LineStyle given as a name-value pair at creation.
    let e = run("contour(peaks(20), 4, 'LineStyle', 'none');");
    assert!(drawn_lines(&e).is_empty());
}

#[test]
fn held_contours_share_one_automatic_clim_that_spans_all_levels() {
    let e = run(
        "[C1, h1] = contour(peaks(20), [-5 0]); hold on; [C2, h2] = contour(peaks(20) + 10, [5 10]); \
         cl = clim; cm = colormap;",
    );
    let (c1, c2) = (matrix(&e, "C1"), matrix(&e, "C2"));
    assert_eq!(num(&e, "cl"), [-5., 10.]);
    let map = matrix(&e, "cm");
    let lines = drawn_lines(&e);
    let (l1, l2) = (levels_with_lines(&c1), levels_with_lines(&c2));
    assert_eq!(lines.len(), l1.len() + l2.len());
    // Both contours map their levels through the shared limits: the first one
    // was redrawn when the second widened them.
    for (s, level) in lines.iter().zip(l1.iter().chain(&l2)) {
        assert_eq!(s.color, mapped(&map, [-5., 10.], *level), "level {level}");
    }
    let distinct: std::collections::BTreeSet<[u8; 3]> = lines.iter().map(|s| s.color).collect();
    assert_eq!(distinct.len(), lines.len(), "one colour per level");
    // A later change of one contour's levels moves the shared limits again.
    let mut e = e;
    e.execute("set(h1, 'LevelList', [-3 3]); cl2 = clim;")
        .unwrap();
    assert_eq!(num(&e, "cl2"), [-3., 10.]);
    let lines = drawn_lines(&e);
    for s in &lines {
        let level_colors = [-3., 3., 5., 10.].map(|l| mapped(&map, [-3., 10.], l));
        assert!(level_colors.contains(&s.color), "{:?}", s.color);
    }
    // Three charts on the axes: still one contour per handle.
    e.execute("[~, h3] = contourf(peaks(20) - 20, 3); L3 = h3.LevelList; cl3 = clim;")
        .unwrap();
    assert_eq!(num(&e, "cl3"), [num(&e, "L3")[0], 10.]);
}

#[test]
fn an_explicit_clim_stays_manual_for_every_contour() {
    let e = run(
        "[~, h1] = contour(peaks(20), [-5 0]); clim([-1 1]); hold on; [C2, h2] = contour(peaks(20) + 10, [5 10]); \
         cl = clim; cm = colormap;",
    );
    assert_eq!(num(&e, "cl"), [-1., 1.]);
    let map = matrix(&e, "cm");
    let lines = drawn_lines(&e);
    // The first contour kept its colours and the second maps through [-1 1] (clamped).
    let first = levels_with_lines(&matrix(&e, "C2")).len();
    let n = lines.len() - first;
    // The first contour keeps the colours it had under its own limits [-5 0].
    assert_eq!(lines[0].color, mapped(&map, [-5., 0.], -5.));
    assert_eq!(lines[1].color, mapped(&map, [-5., 0.], 0.));
    assert!(
        lines[n..]
            .iter()
            .all(|s| s.color == mapped(&map, [-1., 1.], 10.))
    );
    // set(ax, 'CLim', ...) is explicit too, and a LevelList change keeps it.
    let e = run(
        "[~, h1] = contour(peaks(20), [-5 0]); hold on; [~, h2] = contour(peaks(20) + 10, [5 10]); \
         set(gca, 'CLim', [0 2]); set(h2, 'LevelList', [4 8]); cl = clim;",
    );
    assert_eq!(num(&e, "cl"), [0., 2.]);
    // Back to automatic: the next contour spans everything again.
    let e = run(
        "[~, h1] = contour(peaks(20), [-5 0]); hold on; clim([0 2]); clim('auto'); [~, h2] = contour(peaks(20) + 10, [5 10]); cl = clim;",
    );
    assert_eq!(num(&e, "cl"), [-5., 10.]);
}

#[test]
fn contourc_is_a_plain_function_with_one_output_and_normal_display() {
    let mut e = Engine::default();
    let shown = e
        .execute("contourc([1 2; 3 4], [2.5 2.5])")
        .unwrap()
        .output
        .join("\n");
    assert!(shown.trim_start().starts_with("ans ="), "{shown}");
    assert!(shown.contains("2.5000"), "{shown}");
    assert_eq!(matrix(&e, "ans").rows(), 2);
    assert_eq!(matrix(&e, "ans").cols(), 3);
    // A semicolon still sets ans silently; no figure is drawn.
    let mut e = Engine::default();
    let shown = e.execute("contourc(peaks(5), 2);").unwrap().output;
    assert!(shown.is_empty(), "{shown:?}");
    assert!(matrix(&e, "ans").cols() > 0);
    assert!(e.figure.is_none());
    // Used inside an expression, as before.
    let e = run("n = size(contourc(peaks(5), 2), 1);");
    assert_eq!(num(&e, "n"), [2.]);
    assert!(error_of("[a, b] = contourc(peaks(5));").contains("contourc returns at most 1 output"));
    // contour and contourf stay statements: no display.
    let shown = Engine::default()
        .execute("contour(peaks(5))")
        .unwrap()
        .output;
    assert!(shown.is_empty(), "{shown:?}");
}

fn label_objects(e: &Engine) -> Vec<matrust::plot::TextObject> {
    e.figure.as_ref().unwrap().axes.extras.texts.clone()
}

#[test]
fn labels_follow_the_contours_visibility() {
    let mut e = run("[C, h] = contour(peaks(20), [-2 0 2], 'ShowText', 'on');");
    assert!(!label_objects(&e).is_empty());
    assert!(label_objects(&e).iter().all(|t| t.visible));
    e.execute("set(h, 'Visible', 'off'); v = h.Visible;")
        .unwrap();
    assert!(!label_objects(&e).is_empty());
    assert!(
        label_objects(&e).iter().all(|t| !t.visible),
        "labels stay visible"
    );
    assert!(
        series_of(&e, PlotKind::Line).iter().all(|s| !s.visible),
        "lines stay visible"
    );
    e.execute("set(h, 'Visible', 'on');").unwrap();
    assert!(label_objects(&e).iter().all(|t| t.visible));
    // Labels added by clabel(C, h) on a hidden contour start hidden.
    e.execute("set(h, 'Visible', 'off'); set(h, 'ShowText', 'off'); t = clabel(C, h);")
        .unwrap();
    assert!(!label_objects(&e).is_empty());
    assert!(label_objects(&e).iter().all(|t| !t.visible));
    // The exported figure draws neither.
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(
        !svg.contains("data-matrust-text"),
        "hidden labels in the SVG"
    );
}

#[test]
fn clabel_rebuild_keeps_helper_lines_out_of_the_legend() {
    let e = run("Z = peaks(20); [C, h] = contour(Z); clabel(C, h);");
    let lines = series_of(&e, PlotKind::Line);
    assert!(lines.len() > 2);
    let members = lines.iter().filter(|s| !s.legend_hidden).count();
    assert_eq!(
        members, 1,
        "only the primary contour line is a legend member"
    );
}
